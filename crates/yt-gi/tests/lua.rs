//! Runs every `tests/lua/*.lua` script with `gi` installed. A script fails
//! by raising an error; it can run a main loop for asynchronous checks.
//! GTK needs a display (and the main thread), so this is one test binary
//! running the scripts in turn.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Errors reported from callbacks (Lua functions called from C).
static REPORTED: AtomicUsize = AtomicUsize::new(0);

fn report(context: &str, error: &mlua::Error) {
    REPORTED.fetch_add(1, Ordering::Relaxed);
    println!("     {context}: {error}");
}

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lua");
    let filter = std::env::args().nth(1).filter(|a| !a.starts_with('-'));
    let mut scripts: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/lua")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "lua"))
        .filter(|p| {
            filter
                .as_ref()
                .is_none_or(|f| p.to_string_lossy().contains(f.as_str()))
        })
        .collect();
    scripts.sort();
    yt_gi::set_reporter(report);
    let mut failed = 0;
    for script in &scripts {
        let reported = REPORTED.load(Ordering::Relaxed);
        let name = script.file_stem().unwrap().to_string_lossy();
        // SAFETY: tests may use LuaJIT's ffi.
        let lua = unsafe { mlua::Lua::unsafe_new_with(mlua::StdLib::ALL, mlua::LuaOptions::new()) };
        let result = yt_gi::install(&lua).and_then(|_| {
            lua.load(script.as_path())
                .set_name(format!("@{}", script.display()))
                .exec()
        });
        let result = result.and_then(|()| match REPORTED.load(Ordering::Relaxed) - reported {
            0 => Ok(()),
            n => Err(mlua::Error::runtime(format!("{n} errors in callbacks"))),
        });
        match result {
            Ok(()) => println!("ok   {name}"),
            Err(e) => {
                failed += 1;
                println!("FAIL {name}: {e}");
            }
        }
    }
    println!("{} scripts, {failed} failed", scripts.len());
    if failed > 0 {
        std::process::exit(1);
    }
}
