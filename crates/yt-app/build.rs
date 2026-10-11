fn main() {
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
