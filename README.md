# mohaus [alpha]

mohaus builds Mojo packages as Python packages.

It provides a Rust CLI, a PEP 517/660 build backend, and project scaffolding for
mixed Mojo/Python libraries. Generated projects keep Mojo sources under `src/`
and Python package code under `python/`:

```text
my_project/
├── src/lib.mojo
├── python/my_project/
├── pyproject.toml
└── .mojo-version
```

Generated projects currently pin nightly `mojo==1.0.0b2.dev2026050306` and add
the Modular nightly uv index so isolated `uv build` can resolve the compiler.
When `mohaus` is installed from a local wheel, `mohaus develop` forwards that
wheelhouse to uv so isolated editable builds can resolve `mohaus` before the
first public release. Local Modular checkouts can use `$MOHAUS_MOJO`, `$PATH`,
`$MODULAR_HOME/bin/mojo`, and `--no-build-isolation`.
Compiled Mojo extension targets also get adjacent generated typed `.pyi` stubs
during `mohaus develop`, editable rebuilds, and wheel builds. The stub writer
uses the Mojo binding declarations plus referenced `def` headers, so
`PythonObject` becomes `object` instead of `Any`, and simple `PythonObject(...)`
wrappers are narrowed from their body expressions when the source makes the
runtime shape clear. Set
`[tool.mohaus] generate-stub = false` when you want to own those `.pyi` files
manually under `python-src`.
Use `-v`, `-vv`, or `-vvv` before or after a subcommand to print mohaus
diagnostics and forward matching verbosity into uv/pip-backed installs.
Use `mohaus develop -- <uv pip install args>` to pass installer controls
through to the editable install, for example `--python .venv/bin/python`,
`--refresh-package mohaus`, or `--reinstall-package <name>`.
Use `mohaus add --mojo <path>` for local Mojo include roots, or
`mohaus add --mojo github:owner/repo` to clone a git dependency into
`vendor/<repo>` and append that checkout to `mojo-include-paths`.

```bash
mohaus init monpy ~/workspace/monpy
cd ~/workspace/monpy
uv pip install -e .
python -c "import monpy; print(monpy.passthrough('hello'))"
```

## install from CI

Every push to `main` publishes platform wheels and an sdist to a PEP 503
"simple" index hosted on GitHub Pages. Install the latest commit:

```bash
uv pip install mohaus --index https://aarnphm.github.io/mohaus/simple/
```

The default install ships the Rust pyo3 backend. Add the `[mojo]` extra to
pull in `mohaus-mojo`, the sibling package containing pure-Mojo parity ports
of `mohaus`'s build primitives (toolchain, hashing, scaffold, stubgen). When
both packages are installed and a `mojo` toolchain is reachable, the dispatcher
routes the stable parity primitives through the Mojo `.mojopkg` artifacts;
stubgen still stays on the Rust runtime path while the parity port matures:

```bash
uv pip install 'mohaus[mojo]' --index https://aarnphm.github.io/mohaus/simple/
```

`MOHAUS_DISABLE_MOJO_PARITY=1` keeps the Rust backend on the hot path even when `mohaus-mojo` is installed (useful for differential debugging).

Wheels carry a PEP 440 local-version tag matching the source commit (`mohaus-0.1.0+gabcdef-cp311-abi3-...`) so `uv lock` resolves to the exact build.

## development

See [DEVELOPMENT.md](DEVELOPMENT.md) for the complete workflow across all three toolchains (Rust + maturin, Python + uv workspaces, Mojo). The short version:

```bash
nix develop                                 # rust 1.93, maturin, uv, ruff, ratchet
nix run .#check                             # cargo fmt + clippy + test, ruff, ratchet lint
nix develop -c pre-commit run --all-files
```

Mojo parity ports live under [`src/`](src/README.md) and ship as a sibling `mohaus-mojo` wheel. CI builds them via `mojo package` + `mohaus build`, dogfooding the backend.

## license

mohaus is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).

## acknowledgements

mohaus borrows the mixed-layout ergonomics from [PyO3/maturin](https://github.com/pyo3/maturin), extends a lot of [astral-sh/uv](https://github.com/astral-sh/uv) and adapts that workflow for Mojo packages that expose Python bindings.
