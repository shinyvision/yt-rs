# The challenge solver

`lib.min.js` and `core.min.js` are yt-dlp's challenge solver,
[yt-dlp/ejs](https://github.com/yt-dlp/ejs) 0.8.0, unchanged (the release's
`yt.solver.lib.min.js` and `yt.solver.core.min.js`). It finds the functions in
YouTube's player script that turn a stream URL's `n` parameter and scrambled
signature into what YouTube's servers accept. `../solver.rs` runs it.

The solver is public domain (Unlicense). The library bundles meriyah (ISC)
and astring (MIT); their licenses are in `lib.min.js`.

When YouTube changes its player in a way the solver no longer handles, a
newer release usually does: copy its two files here. Users can also drop them
in `~/.config/yt-rs/solver/` to override the bundled ones without rebuilding.
