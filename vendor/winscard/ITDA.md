# winscard 0.3.3, patched for Itda

Unmodified [winscard](https://crates.io/crates/winscard) 0.3.3 (Devolutions, MIT OR Apache-2.0),
pulled in by ironrdp-connector through sspi, except for one line in `Cargo.toml`: its flate2
dependency asked for the `zlib` (C) backend. flate2 prefers any C backend over `zlib-rs`, so
that one feature made the whole build use C zlib (the system libz on macOS, stock zlib without
SIMD on Windows) for Itda's ZRLE encoding and decoding too. It now asks for `zlib-rs`, the
backend Itda itself selects. winscard only uses flate2 to compress smart-card certificates.

Applied through `[patch.crates-io]` in the root `Cargo.toml`. Drop this copy once winscard
stops forcing a flate2 backend upstream, or when ironrdp moves to a winscard that doesn't.
