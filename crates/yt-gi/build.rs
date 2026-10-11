fn main() {
    // libgirepository-2.0 is part of GLib since 2.80.
    if let Err(e) = pkg_config::Config::new()
        .atleast_version("2.80")
        .probe("girepository-2.0")
    {
        panic!("libgirepository-2.0 (GLib 2.80 or later) is needed: {e}");
    }
}
