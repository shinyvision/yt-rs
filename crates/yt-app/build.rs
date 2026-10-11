use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

fn main() {
    link_mpv();
    bundle_plugins();
}

fn link_mpv() {
    // libmpv, wherever the system keeps it (Homebrew's prefix...). When
    // pkg-config can't answer, link it by name from the linker's own
    // paths: in MSYS2, mpv.pc asks for Lua 5.1, which LuaJIT's lua5.1.pc
    // (version 2.1) doesn't satisfy, and the app needs LuaJIT.
    if let Err(e) = pkg_config::Config::new()
        .atleast_version("2.0")
        .probe("mpv")
    {
        println!("cargo:warning=pkg-config can't place libmpv ({e}); linking it by name");
        println!("cargo:rustc-link-lib=mpv");
    }
}

/// The plugins that come with the app (`plugins/` at the top of the
/// repository), compiled in: `BUNDLED` is every file's path and contents,
/// and `BUNDLED_HASH` names this set of them.
fn bundle_plugins() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    let mut hash = DefaultHasher::new();
    let mut out = String::from("pub static BUNDLED: &[(&str, &[u8])] = &[\n");
    for file in &files {
        let path = file
            .strip_prefix(&root)
            .expect("under the plugins directory")
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        path.hash(&mut hash);
        std::fs::read(file)
            .expect("a bundled plugin's file")
            .hash(&mut hash);
        let absolute = file.to_str().expect("a UTF-8 path");
        writeln!(out, "    ({path:?}, include_bytes!({absolute:?})),").unwrap();
    }
    out.push_str("];\n");
    writeln!(
        out,
        "pub const BUNDLED_HASH: &str = \"{:016x}\";",
        hash.finish()
    )
    .unwrap();
    let dest = Path::new(&std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("bundled.rs");
    std::fs::write(dest, out).expect("writing the bundled plugins' list");
}

/// The files under `dir`, leaving out hidden ones and Python's caches.
fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "__pycache__" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            files.push(path);
        }
    }
}
