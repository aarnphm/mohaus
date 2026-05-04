# mohaus [ALPHA]

mohaus builds and publish Mojo packages as Python packages

It is currently v1 scaffolding: a Rust workspace bootstrapped by maturin, a
PEP 517/660 Python backend, and a CLI that can initialize projects shaped like:

```text
my_project/
├── src/lib.mojo
├── python/my_project/
├── pyproject.toml
└── .mojo-version
```

Default generated projects pin nightly `mojo==1.0.0b2.dev2026050306` and include
the Modular nightly uv index so isolated `uv build` can resolve the compiler.
When the `mohaus` console script was installed from a local wheel, `mohaus
develop` forwards that wheelhouse to uv so the isolated editable build can
resolve `mohaus` before it is published. Local Modular checkouts use
`$MOHAUS_MOJO`, `$PATH`, `$MODULAR_HOME/bin/mojo`, and `--no-build-isolation`.

License metadata is intentionally unset until the project license is chosen.

## development

```bash
nix develop
nix run .#check
nix develop -c pre-commit run --all-files
```

## acknowledgements

This is largely inspired by [PyO3/maturin](https://github.com/pyo3/maturin), but for Mojo.
