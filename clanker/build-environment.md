# Reuse the Prismatic toolchain without installing Rust

Confirmed 2026-11-06: Rust is absent from the host PATH. Prismatic's
`target/.rustc_info.json` points at the toolchain in the sibling Xiri work area:

```sh
export PATH="$PWD/../xiri/.work/rustup/toolchains/nightly-2026-10-28-x86_64-unknown-linux-gnu/bin:$PATH"
export CARGO_HOME=/tmp/helios-cargo
cargo test --locked --offline
```

This invokes compiler binaries directly and leaves the sibling checkout and
toolchain unmodified. The compiler reports Rust 1.101.0-nightly, commit
`41089d07a` (2026-10-27). Helios does not require nightly features. Cargo's cache
is temporary, while build output stays in Helios's ignored target/ directory.
No container is required. Recheck the path if the local Xiri work area changes.

