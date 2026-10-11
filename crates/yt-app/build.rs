fn main() {
    // libmpv, wherever the system keeps it (Homebrew's prefix, MSYS2's...).
    if let Err(e) = pkg_config::Config::new()
        .atleast_version("2.0")
        .probe("mpv")
    {
        panic!("libmpv (mpv 0.35 or later) is needed: {e}");
    }
}
