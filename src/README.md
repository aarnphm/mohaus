# src/ (mojo)

Parity ports of selected `crates/mohaus-core` and `crates/mohaus-scaffold`
surfaces, written in Mojo. Each module mirrors the public API of its Rust
counterpart so swap-in is mechanical once parity tests sign off.

This directory follows the `src/` convention `mohaus init` generates for
downstream projects. mohaus dogfooding itself relies on this layout looking
exactly like the layout users get scaffolded.

| package | rust source | stdlib coverage | notes |
| --- | --- | --- | --- |
| `mohaus_toolchain` | `crates/mohaus-core/src/toolchain.rs` | `subprocess`, `os.env`, `pathlib`, `os.path` | full native impl, no Python interop |
| `mohaus_hashing` | `crates/mohaus-core/src/editable.rs` (`source_hash`) | `pathlib` for walking; **pure-Mojo SHA256** in `_sha256.mojo` (FIPS 180-4) | byte-equality contract with the Rust `sha2` crate; KAT vectors pinned in `tests/test_sha256_kat.mojo` |
| `mohaus_scaffold` | `crates/mohaus-scaffold/src/lib.rs` | `pathlib`, file IO | templates copied byte-for-byte from `crates/mohaus-scaffold/src/templates/` |
| `mohaus_stubgen` | `crates/mohaus-core/src/stub.rs` | string parsing only | source-level `.pyi` extractor parity; Rust remains the runtime stubgen path |

## parity contract

For a Mojo module to replace its Rust counterpart in `mohaus`:

1. Same fixture corpus runs against both implementations.
2. Outputs match byte-for-byte (hex digests, file names, file contents,
   wheel SHA256, METADATA byte stream).
3. Both implementations live in CI for at least two weeks before the Rust
   crate is removed.
4. Templates and shared fixtures live in exactly one canonical location and
   are referenced from the Mojo packages without duplicating bytes.

## running tests

```bash
mojo run src/tests/test_toolchain.mojo
mojo run src/tests/test_sha256_kat.mojo
mojo run src/tests/test_hashing.mojo
mojo run src/tests/test_scaffold.mojo
mojo run src/tests/test_stubgen.mojo
```

No CPython interop is required: SHA256 is implemented natively in
`mohaus_hashing/_sha256.mojo`, the toolchain probe uses Mojo's
`subprocess` + `os.env`, scaffold templating reads the same template
files the Rust crate ships, and stubgen only parses binding source text.

## what's blocked

`mohaus_config` (TOML parsing), `mohaus_wheel` (ZIP), and `mohaus_sdist`
(tar.gz) need stdlib gaps to close before parity ports become useful. They
are tracked in `.claude/plans/mojo-migration.md` and `.claude/plans/dogfood.md`.
The watcher and CLI parser are deferred to Phase C per the migration plan.
