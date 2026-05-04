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

```bash
mohaus init monpy ~/workspace/monpy
cd ~/workspace/monpy
uv pip install -e .
python -c "import monpy; print(monpy.passthrough('hello'))"
```

## development

```bash
nix develop
nix run .#check
nix develop -c pre-commit run --all-files
```

The flake provides Rust 1.93.0, maturin, uv, Python 3.11, ruff, alejandra,
deadnix, and statix. Individual checks are exposed as `.#fmt`, `.#clippy`,
`.#test`, and `.#ruff`. Entering the dev shell creates `.venv`, installs
`mohaus` editable with dev extras through nix-provided maturin, and installs the
pre-commit hooks generated from the flake.

The dev shell also generates shell completions under `.venv/share`, exports the
zsh, fish, and bash lookup paths, and sources bash completion when the shell
hook is running under bash.

## license

mohaus is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).

## acknowledgements

mohaus borrows the mixed-layout ergonomics from
[PyO3/maturin](https://github.com/pyo3/maturin) and adapts that workflow for
Mojo packages that expose Python bindings.
