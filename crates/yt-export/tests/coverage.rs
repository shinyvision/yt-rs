//! Nothing is privileged: every public function of the app's service code
//! is exported (callable and hookable by plugins) or says why it isn't.
//!
//! Checked from the source: in the service crates' files, each `pub fn`
//! must be in an `#[export]` impl block or carry `#[export(...)]` itself.
//! Methods of data types (a feed page, a stream) are conveniences on data
//! that plugins have as tables, and aren't services: their types are
//! listed in `DATA`, so a new service type can't slip by unnoticed.

use std::path::{Path, PathBuf};

use syn::{Attribute, ImplItem, Item, Type, Visibility};

/// Where service code lives. The UI (`yt-app/src/ui`) is reached through
/// GObject introspection and the component registry instead.
const SERVICE_DIRS: &[&str] = &["crates/yt-core/src", "crates/yt-app/src"];
const NOT_SERVICES: &[&str] = &[
    "crates/yt-app/src/ui",
    "crates/yt-app/src/plugins",
    // The registries plugins use directly (app.run, app.on, app.filter...).
    "crates/yt-app/src/ext",
    // The built-in commands: in the command registry, which plugins call
    // and replace by name.
    "crates/yt-app/src/commands.rs",
    // Plumbing: the async runtime, and the video's rendering and stream
    // protocol inside the player (plugins reach them through mpv).
    "crates/yt-app/src/runtime.rs",
    "crates/yt-app/src/player/render.rs",
    "crates/yt-app/src/player/gl.rs",
    "crates/yt-app/src/player/protocol.rs",
];

/// Data types, whose methods are conveniences on the data.
const DATA: &[&str] = &[
    // yt-core
    "Token",
    "DeviceCode",
    "Video",
    "Shelf",
    "FeedPage",
    "StreamSet",
    "Stream",
    "Caption",
    "Storyboard",
    "LiveHead",
    "SearchFilters",
    "SearchPage",
    "SearchItem",
    "ChannelRef",
    "Subscription",
    "ChannelTab",
    "ChannelPage",
    "Comment",
    "CommentsPage",
    "CommentThread",
    "VoteActions",
    "RichText",
    "Run",
    "Link",
    "ChatMessage",
    "ChatPage",
    "Download",
    "PlaylistPage",
    "Playlist",
    "PlayerScript",
    "Tracking",
    "VideoAction",
    "WatchPage",
    "Chapter",
    "Account",
    "ChatView",
    "Privacy",
    "Challenges",
    "CommentsEntry",
    "VideoOrder",
    // yt-app
    "List",
    "PlayerState",
    "LiveWindow",
    "ChannelKey",
    "Settings",
];

fn has_export(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "export")
    })
}

fn type_name(ty: &Type) -> String {
    match ty {
        Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn check_items(file: &Path, items: &[Item], problems: &mut Vec<String>) {
    for item in items {
        match item {
            Item::Fn(f) if matches!(f.vis, Visibility::Public(_)) && !has_export(&f.attrs) => {
                problems.push(format!("{}: fn {}", file.display(), f.sig.ident));
            }
            Item::Impl(imp) if imp.trait_.is_none() && !has_export(&imp.attrs) => {
                let ty = type_name(&imp.self_ty);
                if DATA.contains(&ty.as_str()) {
                    continue;
                }
                for i in &imp.items {
                    if let ImplItem::Fn(f) = i
                        && matches!(f.vis, Visibility::Public(_))
                    {
                        problems.push(format!("{}: {ty}::{}", file.display(), f.sig.ident));
                    }
                }
            }
            Item::Mod(m)
                if matches!(
                    m.vis,
                    Visibility::Public(_) | Visibility::Restricted(_) | Visibility::Inherited
                ) =>
            {
                // Inline modules (tests excluded).
                let is_test = m.attrs.iter().any(|a| a.path().is_ident("cfg"));
                if let (Some((_, items)), false) = (&m.content, is_test) {
                    check_items(file, items, problems);
                }
            }
            _ => {}
        }
    }
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("a source directory") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_service_function_is_exported_or_skipped() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in SERVICE_DIRS {
        rust_files(&root.join(dir), &mut files);
    }
    files.retain(|f| !NOT_SERVICES.iter().any(|n| f.starts_with(root.join(n))));
    files.retain(|f| f.file_name().is_some_and(|n| n != "main.rs"));
    let mut problems = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).expect("readable source");
        let parsed = syn::parse_file(&source).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let shown = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
        check_items(&shown, &parsed.items, &mut problems);
    }
    assert!(
        problems.is_empty(),
        "public functions that are neither exported nor skipped with a reason \
         (#[export(\"service\")] or #[export(skip = \"why\")]):\n  {}",
        problems.join("\n  ")
    );
}
