//! `#[export("service")]`: exports public functions through `yt-export`.
//!
//! On an `impl` block, every `pub fn` in it is exported (`&self` methods
//! and associated functions); on a function, that function. Each becomes:
//!
//! - the original body, as a hidden `__export_inner_<name>`,
//! - the public function, which checks its hook point and, if hooked,
//!   runs the hooks around the original with the arguments as JSON,
//! - an entry in `yt_export::FUNCTIONS` with a positional invoker.
//!
//! A function that can't be exported (it takes a closure, returns a
//! reference...) needs `#[export(skip = "why")]`: the macro refuses it
//! otherwise, so every exception is written down. `#[export("svc", local)]`
//! marks a main-thread service, whose async functions aren't `Send`.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::{ToTokens, format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{
    Attribute, Expr, FnArg, GenericArgument, ImplItem, ImplItemFn, Item, ItemFn, ItemImpl, Lit,
    LitStr, Meta, Pat, PathArguments, ReturnType, Signature, Token, Type, Visibility,
};

struct Options {
    service: Option<String>,
    local: bool,
    skip: Option<String>,
}

impl Parse for Options {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut options = Options {
            service: None,
            local: false,
            skip: None,
        };
        while !input.is_empty() {
            if input.peek(LitStr) {
                options.service = Some(input.parse::<LitStr>()?.value());
            } else {
                let name: syn::Ident = input.parse()?;
                match name.to_string().as_str() {
                    "local" => options.local = true,
                    "skip" => {
                        input.parse::<Token![=]>()?;
                        options.skip = Some(input.parse::<LitStr>()?.value());
                    }
                    other => {
                        return Err(syn::Error::new(
                            name.span(),
                            format!("unknown option {other}"),
                        ));
                    }
                }
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(options)
    }
}

#[proc_macro_attribute]
pub fn export(attr: TokenStream, item: TokenStream) -> TokenStream {
    let options = syn::parse_macro_input!(attr as Options);
    let item = syn::parse_macro_input!(item as Item);
    let result = match (&options.skip, item) {
        // A function skipped on its own: left as it is (the attribute
        // documents why it isn't exported).
        (Some(_), item) => Ok(item.into_token_stream()),
        (None, Item::Impl(imp)) => export_impl(&options, imp),
        (None, Item::Fn(f)) => export_free(&options, f),
        (None, other) => Err(syn::Error::new(
            other.span(),
            "#[export] goes on an impl block or a function",
        )),
    };
    result.unwrap_or_else(|e| e.to_compile_error()).into()
}

fn service(options: &Options, span: Span) -> syn::Result<String> {
    options
        .service
        .clone()
        .ok_or_else(|| syn::Error::new(span, "#[export(\"service\")] needs the service's name"))
}

/// `#[export(skip = "...")]` on a function inside an exported impl.
fn take_skip(attrs: &mut Vec<Attribute>) -> syn::Result<Option<String>> {
    let mut skip = None;
    let mut error = None;
    attrs.retain(|a| {
        if !a.path().is_ident("export") {
            return true;
        }
        match a.parse_args::<Options>() {
            Ok(o) if o.skip.is_some() => skip = o.skip,
            Ok(_) => {
                error = Some(syn::Error::new(
                    a.span(),
                    "inside an exported impl, only #[export(skip = \"why\")]",
                ))
            }
            Err(e) => error = Some(e),
        }
        false
    });
    match error {
        Some(e) => Err(e),
        None => Ok(skip),
    }
}

fn export_impl(options: &Options, mut imp: ItemImpl) -> syn::Result<Tokens> {
    let service = service(options, imp.span())?;
    if !imp.generics.params.is_empty() || imp.trait_.is_some() {
        return Err(syn::Error::new(
            imp.span(),
            "#[export] goes on an inherent impl without generics",
        ));
    }
    let self_ty = (*imp.self_ty).clone();
    let mut items = Vec::new();
    for item in std::mem::take(&mut imp.items) {
        let ImplItem::Fn(mut f) = item else {
            items.push(item);
            continue;
        };
        if take_skip(&mut f.attrs)?.is_some() || !matches!(f.vis, Visibility::Public(_)) {
            items.push(ImplItem::Fn(f));
            continue;
        }
        let generated = method(options, &service, &self_ty, f)?;
        for g in generated {
            items.push(ImplItem::Fn(g));
        }
    }
    imp.items = items;
    Ok(imp.into_token_stream())
}

/// How a parameter is passed to the original, from its owned form.
enum Pass {
    Value,
    Ref,
    Str,
    Slice,
    /// `&[&str]`, from a `Vec<String>`.
    StrSlice,
    Path,
    OptionStr,
    OptionRef,
}

struct Param {
    name: syn::Ident,
    ty: Type,
    owned: Type,
    pass: Pass,
}

fn unsupported(span: Span, what: &str) -> syn::Error {
    syn::Error::new(
        span,
        format!(
            "{what} can't cross to plugins as data: mark the function #[export(skip = \"why\")]"
        ),
    )
}

fn has_impl_or_ref(ty: &Type) -> bool {
    let text = ty.to_token_stream().to_string();
    text.contains('&') || text.contains("impl ") || text.contains("dyn ") || text.starts_with("fn")
}

fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(p) = ty else { return None };
    let last = p.path.segments.last()?;
    if last.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    match args.args.first()? {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    }
}

fn param(arg: &FnArg) -> syn::Result<Param> {
    let FnArg::Typed(pt) = arg else {
        return Err(unsupported(arg.span(), "this receiver"));
    };
    let Pat::Ident(id) = &*pt.pat else {
        return Err(unsupported(pt.pat.span(), "a pattern parameter"));
    };
    let ty = (*pt.ty).clone();
    let (owned, pass) = match &ty {
        Type::Reference(r) if r.mutability.is_some() => {
            return Err(unsupported(ty.span(), "a &mut parameter"));
        }
        Type::Reference(r) => match &*r.elem {
            Type::Path(p) if p.path.is_ident("str") => {
                (syn::parse_quote!(::std::string::String), Pass::Str)
            }
            Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Path") => {
                (syn::parse_quote!(::std::path::PathBuf), Pass::Path)
            }
            Type::Slice(s) if s.elem.to_token_stream().to_string() == "& str" => (
                syn::parse_quote!(::std::vec::Vec<::std::string::String>),
                Pass::StrSlice,
            ),
            Type::Slice(s) if !has_impl_or_ref(&s.elem) => {
                let elem = &s.elem;
                (syn::parse_quote!(::std::vec::Vec<#elem>), Pass::Slice)
            }
            elem if !has_impl_or_ref(elem) => (elem.clone(), Pass::Ref),
            _ => return Err(unsupported(ty.span(), "this parameter")),
        },
        t => match option_inner(t) {
            Some(Type::Reference(r)) if r.mutability.is_none() => match &*r.elem {
                Type::Path(p) if p.path.is_ident("str") => (
                    syn::parse_quote!(::std::option::Option<::std::string::String>),
                    Pass::OptionStr,
                ),
                elem if !has_impl_or_ref(elem) => (
                    syn::parse_quote!(::std::option::Option<#elem>),
                    Pass::OptionRef,
                ),
                _ => return Err(unsupported(ty.span(), "this parameter")),
            },
            _ if has_impl_or_ref(t) => return Err(unsupported(ty.span(), "this parameter")),
            _ => (t.clone(), Pass::Value),
        },
    };
    Ok(Param {
        name: id.ident.clone(),
        ty,
        owned,
        pass,
    })
}

/// The expression passing owned `v` the way the original takes it.
fn pass(p: &Param) -> Tokens {
    let v = &p.name;
    match p.pass {
        Pass::Value => quote!(#v),
        Pass::Ref => quote!(&#v),
        Pass::Str => quote!(#v.as_str()),
        Pass::Slice => quote!(#v.as_slice()),
        Pass::Path => quote!(#v.as_path()),
        Pass::StrSlice => {
            quote!(&#v.iter().map(::std::string::String::as_str).collect::<::std::vec::Vec<&str>>())
        }
        Pass::OptionStr => quote!(#v.as_deref()),
        Pass::OptionRef => quote!(#v.as_ref()),
    }
}

fn doc(attrs: &[Attribute]) -> String {
    let mut lines = Vec::new();
    for a in attrs {
        if let Meta::NameValue(nv) = &a.meta
            && nv.path.is_ident("doc")
            && let Expr::Lit(lit) = &nv.value
            && let Lit::Str(s) = &lit.lit
        {
            lines.push(s.value().trim().to_owned());
        }
    }
    lines.join("\n")
}

fn type_text(ty: &Tokens) -> String {
    ty.to_string()
        .replace(" < ", "<")
        .replace("< ", "<")
        .replace(" >", ">")
        .replace("& ", "&")
        .replace(" ,", ",")
        .replace(" :: ", "::")
        .replace(":: ", "::")
}

struct Shape {
    params: Vec<Param>,
    is_async: bool,
    /// Returns `Result<_, _>`: hooks can fail it.
    fallible: bool,
    returns: Tokens,
}

fn shape(sig: &Signature) -> syn::Result<Shape> {
    if !sig.generics.params.is_empty() {
        return Err(unsupported(sig.generics.span(), "a generic function"));
    }
    let params = sig
        .inputs
        .iter()
        .filter(|a| matches!(a, FnArg::Typed(_)))
        .map(param)
        .collect::<syn::Result<Vec<_>>>()?;
    let (fallible, returns) = match &sig.output {
        ReturnType::Default => (false, quote!(())),
        ReturnType::Type(_, ty) => {
            if has_impl_or_ref(ty) {
                return Err(unsupported(ty.span(), "this return type"));
            }
            let fallible = matches!(&**ty, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Result"));
            (fallible, ty.to_token_stream())
        }
    };
    Ok(Shape {
        params,
        is_async: sig.asyncness.is_some(),
        fallible,
        returns,
    })
}

/// The pieces both methods and free functions share.
struct Generated {
    /// Statics and the hook check, at the start of the public function.
    prologue: Tokens,
    /// The invoker's body: takes positional `__args`, calls the public
    /// function through `#call_path`.
    invoker: Tokens,
    invoker_returns: Tokens,
}

#[allow(clippy::too_many_arguments)]
fn generate(
    options: &Options,
    name: &str,
    sig: &Signature,
    attrs: &[Attribute],
    shape: &Shape,
    inner_call: Tokens,
    public_call: Tokens,
    // How the invoker gets the instance: `instance::<T>()` and so on.
    instance: Option<Tokens>,
    invoker_path: Tokens,
) -> Generated {
    let doc = doc(attrs);
    let returns_text = type_text(&shape.returns);
    let param_meta = shape.params.iter().map(|p| {
        let n = p.name.to_string();
        let t = type_text(&p.ty.to_token_stream());
        quote!(::yt_export::Param { name: #n, ty: #t })
    });
    let call_kind = match (shape.is_async, options.local) {
        (false, _) => quote!(Sync),
        (true, false) => quote!(Async),
        (true, true) => quote!(Local),
    };
    let to_args = shape.params.iter().map(|p| {
        let n = p.name.to_string();
        let v = &p.name;
        quote!((#n, ::yt_export::to_value(&#v)))
    });
    let from_named = shape.params.iter().map(|p| {
        let n = p.name.to_string();
        let v = &p.name;
        let owned = &p.owned;
        quote!(let #v: #owned = ::yt_export::arg(&mut __args, #n)?;)
    });
    let from_positional: Vec<Tokens> = shape
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let n = p.name.to_string();
            let v = &p.name;
            let owned = &p.owned;
            quote!(let #v: #owned = ::yt_export::positional(&mut __args, #i, #n)?;)
        })
        .collect();
    let passed: Vec<Tokens> = shape.params.iter().map(pass).collect();
    let outcome = if shape.fallible {
        quote!(::yt_export::Outcome)
    } else {
        quote!(::yt_export::Plain)
    };
    let to_hook = if shape.fallible {
        quote!(#outcome::to_hook)
    } else {
        quote!((|v| #outcome::to_hook(&v)))
    };
    let call_inner = if shape.is_async {
        quote!(#to_hook(#inner_call(#(#passed),*).await))
    } else {
        quote!(#to_hook(#inner_call(#(#passed),*)))
    };
    let run = if shape.is_async {
        quote! {
            ::yt_export::call_async(&__HOOK, __args, |mut __args: ::yt_export::Value| async move {
                #(#from_named)*
                #call_inner
            }).await
        }
    } else {
        quote! {
            ::yt_export::call_sync(&__HOOK, __args, &mut |mut __args: ::yt_export::Value| {
                #(#from_named)*
                #call_inner
            })
        }
    };
    let finish = if shape.fallible {
        quote!(return ::yt_export::Outcome::from_hook(__result);)
    } else {
        quote! {
            match ::yt_export::Plain::from_hook(__result) {
                Ok(v) => return v,
                Err(e) => ::yt_export::report(#name, &e),
            }
        }
    };
    let prologue = quote! {
        static __HOOK: ::yt_export::HookPoint = ::yt_export::HookPoint::new(#name);
        #[::yt_export::linkme::distributed_slice(::yt_export::FUNCTIONS)]
        #[linkme(crate = ::yt_export::linkme)]
        static __FUNCTION: ::yt_export::Function = ::yt_export::Function {
            name: #name,
            doc: #doc,
            params: &[#(#param_meta),*],
            returns: #returns_text,
            call: ::yt_export::Call::#call_kind(#invoker_path),
            hook: &__HOOK,
        };
        if __HOOK.active() {
            match ::yt_export::args(::std::vec![#(#to_args),*]) {
                Ok(__args) => {
                    let __result = #run;
                    #finish
                }
                Err(e) => ::yt_export::report(#name, &e),
            }
        }
    };
    let this = match &instance {
        Some(get) => quote!(let __this = #get?;),
        None => quote!(),
    };
    let public_passed = passed.clone();
    let (invoker, invoker_returns) = match (shape.is_async, options.local) {
        (false, _) => (
            quote! {
                #this
                #(#from_positional)*
                #to_hook(#public_call(#(#public_passed),*))
            },
            quote!(::std::result::Result<::yt_export::Value, ::std::string::String>),
        ),
        (true, local) => {
            let future = if local {
                quote!(LocalFuture)
            } else {
                quote!(SendFuture)
            };
            (
                quote! {
                    #this
                    #(#from_positional)*
                    ::std::result::Result::Ok(::std::boxed::Box::pin(async move {
                        #to_hook(#public_call(#(#public_passed),*).await)
                    }))
                },
                quote!(::std::result::Result<::yt_export::#future, ::std::string::String>),
            )
        }
    };
    let _ = sig;
    Generated {
        prologue,
        invoker,
        invoker_returns,
    }
}

/// Keeps a signature's parameters but drops `mut` (the public function
/// only passes them on).
fn plain_signature(sig: &Signature) -> Signature {
    let mut sig = sig.clone();
    for arg in sig.inputs.iter_mut() {
        if let FnArg::Typed(pt) = arg
            && let Pat::Ident(id) = &mut *pt.pat
        {
            id.mutability = None;
        }
    }
    sig
}

fn forwarded_args(sig: &Signature) -> Vec<Tokens> {
    sig.inputs
        .iter()
        .filter_map(|a| match a {
            FnArg::Typed(pt) => match &*pt.pat {
                Pat::Ident(id) => {
                    let n = &id.ident;
                    Some(quote!(#n))
                }
                _ => None,
            },
            FnArg::Receiver(_) => None,
        })
        .collect()
}

fn method(
    options: &Options,
    service: &str,
    self_ty: &Type,
    f: ImplItemFn,
) -> syn::Result<Vec<ImplItemFn>> {
    let fn_name = f.sig.ident.clone();
    let name = format!("{service}.{fn_name}");
    let receiver = f.sig.receiver();
    let shape = shape(&f.sig)?;
    // How the invoker gets the instance: `&self` methods take whichever
    // kind is provided (an Arc for futures that run anywhere);
    // `self: &Rc<Self>` and `self: &Arc<Self>` the one they need.
    let getter = |kind: &str| {
        let kind = format_ident!("{}", kind);
        quote!(::yt_export::#kind::<#self_ty>())
    };
    let instance = match receiver {
        None => None,
        Some(r) if r.reference.is_some() && r.mutability.is_none() && r.colon_token.is_none() => {
            Some(getter(match (shape.is_async, options.local) {
                (false, _) => "instance",
                (true, false) => "instance_arc",
                (true, true) => "instance_rc",
            }))
        }
        Some(r) if r.colon_token.is_some() => {
            let text = r.ty.to_token_stream().to_string().replace(' ', "");
            match text.as_str() {
                "&Rc<Self>" => Some(getter("instance_rc")),
                "&Arc<Self>" => Some(getter("instance_arc")),
                _ => {
                    return Err(unsupported(
                        r.span(),
                        "this receiver (&self, &Rc<Self> and &Arc<Self> are)",
                    ));
                }
            }
        }
        Some(r) => {
            return Err(unsupported(
                r.span(),
                "this receiver (&self, &Rc<Self> and &Arc<Self> are)",
            ));
        }
    };
    let has_self = instance.is_some();
    let inner = format_ident!("__export_inner_{}", fn_name);
    let invoker = format_ident!("__export_call_{}", fn_name);
    let inner_path = if has_self {
        quote!(self.#inner)
    } else {
        quote!(Self::#inner)
    };
    let public_path = if has_self {
        quote!(__this.#fn_name)
    } else {
        quote!(#self_ty::#fn_name)
    };
    let invoker_path = quote!(#self_ty::#invoker);
    let generated = generate(
        options,
        &name,
        &f.sig,
        &f.attrs,
        &shape,
        inner_path,
        public_path,
        instance,
        invoker_path,
    );
    let other_attrs: Vec<&Attribute> = f
        .attrs
        .iter()
        .filter(|a| !a.path().is_ident("doc"))
        .collect();
    let mut inner_sig = f.sig.clone();
    inner_sig.ident = inner.clone();
    let block = &f.block;
    let inner_fn: ImplItemFn = syn::parse_quote! {
        #[doc(hidden)]
        #(#other_attrs)*
        #inner_sig #block
    };
    let public_sig = plain_signature(&f.sig);
    let attrs = &f.attrs;
    let vis = &f.vis;
    let prologue = &generated.prologue;
    let forwarded = forwarded_args(&f.sig);
    let await_ = if shape.is_async {
        quote!(.await)
    } else {
        quote!()
    };
    let tail = if has_self {
        quote!(self.#inner(#(#forwarded),*)#await_)
    } else {
        quote!(Self::#inner(#(#forwarded),*)#await_)
    };
    let public_fn: ImplItemFn = syn::parse_quote! {
        #(#attrs)*
        #vis #public_sig {
            #prologue
            #tail
        }
    };
    let invoker_body = &generated.invoker;
    let invoker_returns = &generated.invoker_returns;
    let invoker_fn: ImplItemFn = syn::parse_quote! {
        #[doc(hidden)]
        #[allow(unused_mut)]
        fn #invoker(mut __args: ::std::vec::Vec<::yt_export::Value>) -> #invoker_returns {
            #invoker_body
        }
    };
    Ok(vec![public_fn, inner_fn, invoker_fn])
}

fn export_free(options: &Options, f: ItemFn) -> syn::Result<Tokens> {
    let service = service(options, f.span())?;
    if !matches!(f.vis, Visibility::Public(_)) {
        return Err(syn::Error::new(
            f.sig.span(),
            "only public functions are exported",
        ));
    }
    let fn_name = f.sig.ident.clone();
    let name = format!("{service}.{fn_name}");
    let shape = shape(&f.sig)?;
    let inner = format_ident!("__export_inner_{}", fn_name);
    let invoker = format_ident!("__export_call_{}", fn_name);
    let generated = generate(
        options,
        &name,
        &f.sig,
        &f.attrs,
        &shape,
        quote!(#inner),
        quote!(#fn_name),
        None,
        quote!(#invoker),
    );
    let other_attrs: Vec<&Attribute> = f
        .attrs
        .iter()
        .filter(|a| !a.path().is_ident("doc"))
        .collect();
    let mut inner_sig = f.sig.clone();
    inner_sig.ident = inner.clone();
    let block = &f.block;
    let public_sig = plain_signature(&f.sig);
    let attrs = &f.attrs;
    let vis = &f.vis;
    let prologue = &generated.prologue;
    let forwarded = forwarded_args(&f.sig);
    let await_ = if shape.is_async {
        quote!(.await)
    } else {
        quote!()
    };
    let invoker_body = &generated.invoker;
    let invoker_returns = &generated.invoker_returns;
    Ok(quote! {
        #(#attrs)*
        #vis #public_sig {
            #prologue
            #inner(#(#forwarded),*)#await_
        }

        #[doc(hidden)]
        #(#other_attrs)*
        #inner_sig #block

        #[doc(hidden)]
        #[allow(unused_mut)]
        fn #invoker(mut __args: ::std::vec::Vec<::yt_export::Value>) -> #invoker_returns {
            #invoker_body
        }
    })
}
