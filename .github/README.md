# mohaus [alpha]

mohaus builds Mojo packages as Python packages.

It provides a Rust CLI, a PEP 517/660 build backend, and project scaffolding for
mixed Mojo/Python libraries. Generated projects keep Mojo sources under `src/`
and Python package code under `python/`:

```text
my_project/
├── src/lib.mojo
├── python/my_project/
├── flake.nix
├── pyproject.toml
├── .gitignore
└── .gitattributes
```

```bash
mohaus init monpy ~/workspace/monpy
# or pin the scaffold explicitly:
mohaus init monpy ~/workspace/monpy --mojo-version 1.0.0b1
cd ~/workspace/monpy
uv pip install -e .
python -c "import monpy; print(monpy.passthrough('hello'))"
```

Nightly Modular wheels work through installer passthrough:

```bash
uv pip install -e . --prerelease allow --extra-index-url https://whl.modular.com/nightly/simple/
```

## install from CI

Every push to `main` publishes platform wheels and an sdist to a PEP 503 "simple" index hosted on GitHub Pages. Install the latest commit:

```bash
uv pip install mohaus --index https://aarnphm.github.io/mohaus/simple/
```

The default install ships the Rust pyo3 backend.

Add the `[mojo]` extra to pull in `mohaus-mojo`, the sibling package containing pure-Mojo parity ports of `mohaus`'s build primitives (toolchain, hashing, scaffold, stubgen).

When both packages are installed and a `mojo` toolchain is reachable, the dispatcher routes the stable parity primitives through the Mojo `.mojopkg` artifacts.

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
