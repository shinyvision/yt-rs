//! Exported functions: listed, callable with JSON, and hookable.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::{Value, json};
use yt_export::{Call, Interceptor, Remote, Step, export};

pub struct Counter {
    n: AtomicU32,
}

#[export("counter")]
impl Counter {
    /// Adds to the count; returns the new count.
    pub fn add(&self, by: u32) -> u32 {
        self.n.fetch_add(by, Ordering::SeqCst) + by
    }

    pub fn greet(name: &str, punctuation: Option<&str>) -> String {
        format!("hi {name}{}", punctuation.unwrap_or(""))
    }

    pub fn total(values: &[u32]) -> u32 {
        values.iter().sum()
    }

    pub async fn shout(&self, text: &str) -> Result<String, String> {
        if text.is_empty() {
            return Err("nothing to shout".into());
        }
        Ok(text.to_uppercase())
    }

    #[export(skip = "takes a closure")]
    pub fn each(&self, f: impl Fn(u32)) {
        f(self.n.load(Ordering::SeqCst));
    }

    fn private_helper(&self) -> u32 {
        1
    }
}

#[export("misc")]
pub fn double(mut x: i64) -> i64 {
    x *= 2;
    x
}

/// Hooks for the test: they add one to `by`/`x`, and append "!" to
/// strings returned; `counter.shout` hooks run on another thread.
struct TestHooks;

fn tweak_args(name: &str, mut args: Value) -> Value {
    match name {
        "counter.add" => args["by"] = json!(args["by"].as_u64().unwrap() + 1),
        "misc.double" => args["x"] = json!(args["x"].as_i64().unwrap() + 1),
        "counter.shout" => args["text"] = json!(format!("{}?", args["text"].as_str().unwrap())),
        _ => {}
    }
    args
}

fn tweak_result(result: Result<Value, String>) -> Result<Value, String> {
    match result {
        Ok(Value::String(s)) => Ok(json!(format!("{s}!"))),
        other => other,
    }
}

impl Interceptor for TestHooks {
    fn is_home(&self) -> bool {
        true
    }

    fn run(
        &self,
        name: &'static str,
        args: Value,
        next: &mut dyn FnMut(Value) -> Result<Value, String>,
    ) -> Result<Value, String> {
        tweak_result(next(tweak_args(name, args)))
    }

    fn start(&self, name: &'static str, args: Value, mut remote: Remote) {
        std::thread::spawn(move || {
            (remote.send)(Step::Next(tweak_args(name, args)));
            let result = remote.results.blocking_recv().expect("a result");
            (remote.send)(Step::Done(tweak_result(result)));
        });
    }
}

fn call(name: &str, args: Vec<Value>) -> Result<Value, String> {
    match &yt_export::find(name).expect(name).call {
        Call::Sync(f) => f(args),
        _ => panic!("{name} is async"),
    }
}

#[test]
fn everything() {
    yt_export::provide_arc::<Counter>(|| {
        static COUNTER: std::sync::OnceLock<Arc<Counter>> = std::sync::OnceLock::new();
        Some(
            COUNTER
                .get_or_init(|| {
                    Arc::new(Counter {
                        n: AtomicU32::new(0),
                    })
                })
                .clone(),
        )
    });
    yt_export::set_interceptor(Box::new(TestHooks));
    let counter = yt_export::instance_arc::<Counter>().unwrap();

    // Listed, with what they take.
    let names: Vec<&str> = yt_export::functions().iter().map(|f| f.name).collect();
    for n in [
        "counter.add",
        "counter.greet",
        "counter.total",
        "counter.shout",
        "misc.double",
    ] {
        assert!(names.contains(&n), "{n} is listed: {names:?}");
    }
    assert!(!names.contains(&"counter.each"), "skipped");
    assert!(!names.contains(&"counter.private_helper"), "private");
    let add = yt_export::find("counter.add").unwrap();
    assert_eq!(add.doc, "Adds to the count; returns the new count.");
    assert_eq!(add.params[0].name, "by");
    assert_eq!(add.params[0].ty, "u32");
    assert_eq!(add.returns, "u32");

    // Callable with JSON, unhooked.
    assert_eq!(call("counter.add", vec![json!(2)]), Ok(json!(2)));
    assert_eq!(
        call("counter.greet", vec![json!("ann"), json!(".")]),
        Ok(json!("hi ann."))
    );
    assert_eq!(
        call("counter.greet", vec![json!("bob")]),
        Ok(json!("hi bob"))
    );
    assert_eq!(call("counter.total", vec![json!([1, 2, 3])]), Ok(json!(6)));
    assert_eq!(call("misc.double", vec![json!(4)]), Ok(json!(8)));
    assert!(
        call("counter.add", vec![json!("x")])
            .unwrap_err()
            .contains("by")
    );

    // Hooked: from Rust and from JSON calls alike.
    yt_export::set_hooked("counter.add", true);
    assert_eq!(counter.add(1), 4, "the hook added one to `by`");
    assert_eq!(call("counter.add", vec![json!(1)]), Ok(json!(6)));
    yt_export::set_hooked("counter.add", false);
    assert_eq!(counter.add(1), 7, "unhooked again");
    yt_export::set_hooked("misc.double", true);
    assert_eq!(double(4), 10);
    yt_export::set_hooked("counter.greet", true);
    assert_eq!(Counter::greet("cy", None), "hi cy!");

    // Async, with hooks running elsewhere.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    assert_eq!(rt.block_on(counter.shout("hey")), Ok("HEY".into()));
    yt_export::set_hooked("counter.shout", true);
    assert_eq!(rt.block_on(counter.shout("hey")), Ok("HEY?!".into()));
    let Call::Async(f) = &yt_export::find("counter.shout").unwrap().call else {
        panic!()
    };
    assert_eq!(rt.block_on(f(vec![json!("a")]).unwrap()), Ok(json!("A?!")));
    assert_eq!(
        rt.block_on(counter.shout("")),
        Ok("?!".into()),
        "the hook made the text valid"
    );
    // An error from the original reaches the caller through the hooks.
    yt_export::set_hooked("counter.shout", false);
    assert_eq!(
        rt.block_on(counter.shout("")),
        Err("nothing to shout".into())
    );
    // Lua's empty tables arrive as empty objects; they still fit lists.
    assert_eq!(call("counter.total", vec![json!({})]), Ok(json!(0)));
    counter.each(|n| assert!(n > 0));
    assert_eq!(counter.private_helper(), 1);
}
