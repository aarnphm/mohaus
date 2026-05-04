# mohaus-mojo

Mojo parity ports of `mohaus`'s build primitives. Installed automatically by
`uv pip install mohaus[mojo]`. Ships compiled `.mojopkg` artifacts plus a
small Python loader that exposes their entry points to `mohaus._dispatch`.

This package is not a standalone tool. It only matters when paired with the
`mohaus` Rust orchestrator, and when a `mojo` toolchain is available on the
host. Without `mojo`, the loader returns `None` and the orchestrator falls
back to its native Rust implementations.

See `.claude/plans/dogfood.md` for the cutover plan.
