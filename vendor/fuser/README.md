# Vendored `fuser` 0.18.0

This directory contains the upstream [`fuser` 0.18.0](https://github.com/cberner/fuser/tree/0.18.0)
crate with one FOD-specific patch: the high-level `Filesystem` trait can receive
`FUSE_INTERRUPT` requests, and the dispatcher acknowledges a handled interrupt
or returns `EAGAIN` when the target request is not registered by the filesystem.

The patch lets FOD interrupt a pending blocking POSIX lock wait. Keep the
remaining crate source aligned with upstream 0.18.0 when updating this copy.
`Cargo.toml` at the repository root selects this source through `[patch.crates-io]`.

The upstream crate is licensed under MIT; see [`LICENSE.md`](LICENSE.md).
