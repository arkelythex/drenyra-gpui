# Patched `xattr 0.2.3`

Vendored copy of [`xattr` 0.2.3](https://crates.io/crates/xattr/0.2.3) (MIT OR Apache-2.0, licenses
included) used through `[patch.crates-io]` in the root `Cargo.toml`.

**Why:** `gpui 0.2.2` → `gpui_http_client` → `zed-async-tar 0.5.0-zed` requires `xattr ^0.2`.
`libc >= 0.2.191` removed the Linux `ENOATTR` alias, so `xattr 0.2.3` no longer compiles on Linux,
and older `libc` versions are ruled out by other GPUI dependencies (`nix`, `mio`).

**Change:** a single constant in `src/sys/mod.rs`. On Linux and Android `ENOATTR` is `libc::ENODATA`
(the value it always aliased); every other platform still uses `libc::ENOATTR`.

**Remove when:** GPUI moves `zed-async-tar` to `xattr >= 1`, or `xattr 0.2.x` ships a fix. Delete
this directory and the `[patch.crates-io]` entry.
