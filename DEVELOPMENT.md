# development

This document is the source of truth for everyday work on mohaus. The repo is
deliberately split across three toolchains; each has a canonical workflow that
the rest of the project assumes you're following. If something feels wrong in
practice, fix the doc first, then the code.

## repository layout

```
mohaus/
├── crates/                         # Rust workspace (managed by cargo + maturin)
│   ├── mohaus-cli/                 # `mohaus` binary
│   ├── mohaus-core/                # build, config, wheel, sdist, editable, pyproject_edit
│   ├── mohaus-pep517/              # pyo3 bindings (cdylib for `mohaus.mohaus_pep517`)
│   └── mohaus-scaffold/            # init/new templates
├── python/                         # uv workspace root for Python packages
│   ├── mohaus/                     # main mohaus Python package (built by maturin)
│   │   ├── __init__.py
│   │   ├── _cli.py
│   │   ├── _dispatch.py
│   │   ├── _editable.py
│   │   └── backend.py
│   └── mohaus_mojo/                # sibling package (built by mohaus, dogfooding)
│       ├── pyproject.toml          # build-backend = "mohaus.backend", pure = true
│       ├── README.md
│       ├── LICENSE
│       ├── __init__.py
│       └── _mojopkg/, templates/   # CI-staged build artifacts (gitignored)
├── src/                            # Mojo parity ports + tests
│   ├── mohaus_toolchain/
│   ├── mohaus_hashing/             # native SHA256 + walker
│   ├── mohaus_scaffold/
│   └── tests/
├── tests/
│   ├── fixtures/                   # corpus shared by Rust integration + Mojo parity
│   ├── parity/                     # cross-implementation diff harnesses
│   └── python/                     # pytest suite (backend shape, dispatch, etc.)
├── scripts/
│   ├── build_simple_index.py       # PEP 503 index emitter (CI)
│   └── stage_mohaus_mojo_assets.py # collects .mojopkgs + templates for the wheel
├── .github/workflows/ci.yml        # ratchet-pinned, see "CI" below
├── flake.nix                       # nix dev shell + apps
└── pyproject.toml                  # mohaus root + [tool.uv.workspace]
```

The three toolchains:
- **Rust** owns the build orchestration, CLI, PEP 517/660 backend, and the
  pyo3 cdylib. Built by `cargo` for tests and by `maturin` for wheels.
- **Python** owns the `mohaus` runtime shim, the dispatcher between Rust and
  Mojo backends, and the `mohaus_mojo` sibling package. Multi-project
  resolution uses **uv workspaces**.
- **Mojo** owns the parity ports of toolchain / hashing / scaffold. Built by
  `mojo package` into `.mojopkg` artifacts that ship inside the
  `mohaus-mojo` wheel.

## environment setup

```bash
nix develop
```

That gives you the pinned Rust 1.93.0 toolchain, maturin, uv, Python 3.11,
ruff, alejandra, deadnix, statix, and ratchet, plus pre-commit hooks
generated from the flake. Entering the dev shell:
- creates `.venv/` and installs `mohaus` editable via maturin (with dev
  extras) so `mohaus`, `pytest`, and the local backend are immediately
  runnable;
- generates shell completions into `.venv/share` and exports them for bash,
  fish, and zsh.

If you can't use Nix, you need: Rust 1.93.0 + clippy/rustfmt, uv, Python
3.11, ruff, ratchet, and (for Mojo work) a Mojo toolchain reachable via
`$PATH` / `$MOHAUS_MOJO` / `$MODULAR_HOME/bin/mojo`.

## rust development (cargo + maturin)

Rust is the canonical implementation. Every crate lives under `crates/` and
shares the workspace `Cargo.toml`. Workflow:

```bash
cargo fmt           # before committing
cargo clippy --workspace --all-targets --all-features
cargo test --workspace
```

The pyo3 cdylib (`mohaus.mohaus_pep517`) is built by **maturin**, not
cargo:

```bash
maturin develop --uv             # editable install into .venv
maturin build --release --out dist
```

`maturin develop` consumes `[tool.maturin]` in the root `pyproject.toml`,
which points at `crates/mohaus-pep517/Cargo.toml` and exposes
`bindings = "pyo3"` with `abi3-py311`. One wheel works across CPython
3.11 / 3.12 / 3.13 because of abi3.

Testing convention:
- Crate-local unit tests live next to the code (`#[cfg(test)] mod tests`).
- Cross-crate integration tests live under `crates/mohaus-core/tests/` —
  see `fixture_args.rs` and `pure_wheel.rs`.
- Anything that needs a real Mojo toolchain belongs in CI's `mojo-smoke` or
  `mojo-parity` jobs, not in `cargo test`.

## python multi-project workflow (uv workspaces)

The root `pyproject.toml` declares:

```toml
[tool.uv.workspace]
members = ["python/mohaus_mojo"]

[tool.uv.sources]
mohaus-mojo = { workspace = true }
```

Two members:
- **`mohaus`** — the root project, built by maturin.
- **`mohaus-mojo`** — `python/mohaus_mojo/pyproject.toml`, built by mohaus
  itself in pure mode.

The default install is Rust-only:
```bash
uv pip install -e .
```

Adding the optional `mojo` extra pulls in the workspace's mohaus-mojo
sibling, which ships the compiled `.mojopkg` parity ports:
```bash
uv pip install -e '.[mojo]'
```

The runtime switch lives at `python/mohaus/_dispatch.py`. It returns
`"mojo"` when both `mohaus_mojo` is importable AND a `mojo` toolchain is
reachable; otherwise `"rust"`. Override at runtime with:
```bash
MOHAUS_DISABLE_MOJO_PARITY=1 python -c "import mohaus._dispatch as d; print(d.active_backend_name())"
```

Adding dependencies:
```bash
mohaus add httpx                         # uv add to [project.dependencies]
mohaus add pytest --group dev            # uv add --group dev
mohaus add 'mojo==1.0.0b2.devXXXX' --build-system   # edit [build-system].requires
mohaus add --mojo vendor/some_pkg        # append to [tool.mohaus.mojo-include-paths]
```

Lint + format:
```bash
uvx ruff format --config "indent-width=2" --config "line-length=119" --config "preview=true" --check python tests scripts
uvx ruff check python tests scripts
```

## mojo development

Mojo source lives under `src/`. The layout intentionally mirrors what
`mohaus init` generates for downstream users — mohaus's parity ports
*are* a real mohaus project, modulo the workspace context.

Each parity package is one directory:
- `src/mohaus_toolchain/` — Mojo equivalent of `mohaus_core::toolchain`.
- `src/mohaus_hashing/` — pure-Mojo SHA256 + walker (mirrors
  `mohaus_core::editable::source_hash`).
- `src/mohaus_scaffold/` — `init`/`new` templating.

Workflow:

```bash
# compile each parity package into a .mojopkg
mojo package src/mohaus_toolchain -o target/mojopkg/mohaus_toolchain.mojopkg
mojo package src/mohaus_hashing  -o target/mojopkg/mohaus_hashing.mojopkg
mojo package src/mohaus_scaffold -o target/mojopkg/mohaus_scaffold.mojopkg

# unit tests for each package
mojo run -I src src/tests/test_toolchain.mojo
mojo run -I src src/tests/test_sha256_kat.mojo
mojo run -I src src/tests/test_hashing.mojo
mojo run -I src src/tests/test_scaffold.mojo
```

Editor support: install the `Mojo (Modular)` VS Code extension. The Mojo
LSP picks up `src/` automatically; for cross-file imports between parity
packages, use `mojo run -I src` or set `MOJO_PYTHON_LIBRARY` if you need
CPython interop.

### parity contract

Each Mojo port mirrors the public API of its Rust counterpart. The
contract is byte-equality on outputs. CI's `mojo-parity` job runs
`tests/parity/run_source_hash_parity.py` after compiling the .mojopkgs:
- Rust path: `mohaus.mohaus_pep517.tree_hash_for_dir` (exposed via pyo3).
- Mojo path: `mojo run -I src tests/parity/_mojo_hash_runner.mojo <fixture>`.

Both implementations hash the same fixture corpus (`tests/fixtures/`) and
the harness asserts identical hex digests. A regression in either
implementation immediately fails CI.

### packaging the mojo parity ports

The `mohaus-mojo` wheel bundles the `.mojopkg` files plus byte-identical
copies of the scaffold templates from `crates/mohaus-scaffold/src/templates/`.
Both directories are CI-staged build artifacts under
`python/mohaus_mojo/_mojopkg/` and `python/mohaus_mojo/templates/`; both
are gitignored.

The flow:
1. `mojo package` produces `.mojopkg` files under `target/mojopkg/`.
2. `scripts/stage_mohaus_mojo_assets.py` copies those `.mojopkg`s plus the
   canonical scaffold templates into `python/mohaus_mojo/`.
3. `cd python/mohaus_mojo && mohaus build --out <repo>/dist` produces a
   `py3-none-any` wheel via mohaus's pure-mode backend.

Locally, after editing a Mojo parity port:
```bash
mojo package src/mohaus_toolchain -o target/mojopkg/mohaus_toolchain.mojopkg
mojo package src/mohaus_hashing  -o target/mojopkg/mohaus_hashing.mojopkg
mojo package src/mohaus_scaffold -o target/mojopkg/mohaus_scaffold.mojopkg
python scripts/stage_mohaus_mojo_assets.py \
  --mojopkg-dir target/mojopkg \
  --templates-source crates/mohaus-scaffold/src/templates \
  --package-root python/mohaus_mojo
(cd python/mohaus_mojo && mohaus build --out "$(git rev-parse --show-toplevel)/dist")
```

The `_mojopkg/` and `templates/` dirs in the working tree are ephemeral
artifacts. If you're done testing locally, `git clean -fd python/mohaus_mojo/`
returns the working tree to canonical source.

## CI

`.github/workflows/ci.yml` is the single workflow. Every action reference
is pinned to a commit SHA via `ratchet`. Job graph:

```
ratchet ─────────► (gates everything; advisory step pinned to a SHA)
rust           ─┐
python-backend ─┼─► wheel-matrix ──► index (main only)
sdist          ─┘   wheel-mojo
mojo-smoke          ↑
mojo-parity ────────┘
```

- **`ratchet`** — runs `ratchet lint .github/workflows/ci.yml` via the
  pinned `ghcr.io/sethvargo/ratchet:0.11.4` Docker image. Fails if any
  action ref is unpinned.
- **`rust`** — `cargo fmt --check`, `cargo clippy --all --all-features`,
  `cargo test --all-features` on linux + macos.
- **`python-backend`** — installs mohaus, runs ruff (format + lint),
  runs `pytest tests/python` for backend shape + dispatch tests.
- **`mojo-smoke`** — installs nightly Mojo, runs `mohaus init demo`,
  builds and imports the demo project end-to-end.
- **`mojo-parity`** — packages the parity ports, runs Mojo unit tests,
  runs the source-hash parity diff against the Rust implementation,
  uploads `.mojopkg` artifacts.
- **`wheel-matrix`** — `PyO3/maturin-action` builds `mohaus` wheels for
  `linux-x86_64`, `linux-aarch64`, `macos-arm64`. macOS x86_64 is
  intentionally absent because GitHub's free `macos-13` runner pool
  routinely queues for 10+ hours and would block the publish path.
  Stamps the commit SHA into `[project] version` as a PEP 440 local-version
  segment so users can pin to specific commits.
- **`sdist`** — `maturin sdist` for `mohaus`; `mohaus sdist` for
  `mohaus-mojo` (dogfooded through the mohaus CLI).
- **`wheel-mojo`** — downloads `.mojopkg` artifacts from `mojo-parity`,
  runs the asset stager, then `mohaus build` produces the `mohaus-mojo`
  pure wheel.
- **`index`** (main only) — collects every artifact and publishes a PEP
  503 simple index to GitHub Pages. Each artifact gets a `#sha256=...`
  fragment for `uv` to verify integrity. Uses
  `if: always() && needs.<job>.result != 'failure'` so a single skipped
  matrix shard never dams the publish path.

### troubleshooting the index

Common failure modes when `https://<owner>.github.io/mohaus/simple/` is
empty or 404:

- **Pages source still set to a branch**: GitHub's old default is to
  publish from `gh-pages`. The Actions-based workflow only works when the
  source is set to "GitHub Actions" (see above).
- **Latest run never reached the `index` job**: any matrix shard hanging
  in `queued` blocked the publish path. The current workflow's `if:`
  guard tolerates skipped shards, but a `failure` still aborts. Check
  `gh run view <id> --json jobs --jq '.jobs[] | "\(.name): \(.conclusion)"'`.
- **Pushed to a non-`main` branch**: `index` is gated on
  `github.ref == 'refs/heads/main'`. Branches don't publish.
- **First push hasn't completed yet**: there are simply no artifacts
  uploaded. Wait for the first green run on `main`.
- **`uv` can't resolve from the index**: re-run with `-v` and look for
  `404` on the `index.html`. Confirm trailing slash:
  `https://aarnphm.github.io/mohaus/simple/` (with the slash, not without).

While Pages is being set up, the artifacts are still reachable via:
```bash
gh run download <run-id>
```
which works for any workflow run regardless of Pages state.

## ratchet workflow

Pin every GitHub Action reference to a commit SHA. Run locally:

```bash
nix run .#ratchet-lint            # fails if any ref is unpinned
nix run .#ratchet-update          # updates all SHAs to the latest released tags
ratchet update .github/workflows/ci.yml   # equivalent without nix
```

Pre-commit also runs `ratchet lint` on every workflow change. To bump pins
across the workflow:
1. `ratchet update .github/workflows/ci.yml`
2. Inspect the diff; SHAs should still resolve to known-good tags.
3. Commit. The `ratchet` CI job validates the result on push.

## release process (planned, not yet wired)

The `index` job publishes per-commit artifacts to GitHub Pages on every
`main` push, but that's a development surface, not PyPI. For an actual
PyPI release:

1. Tag a commit (`v0.1.0`) and push the tag.
2. A separate `release.yml` workflow (TODO) downloads the tagged build's
   artifacts and uses PyPI's trusted-publisher OIDC flow to upload.
3. Both `mohaus` and `mohaus-mojo` get published in lockstep.

We never store a PyPI token in CI secrets.

## quick command reference

| task | command |
| --- | --- |
| Enter dev shell | `nix develop` |
| Format Rust | `cargo fmt` |
| Rust lint | `nix run .#clippy` |
| Rust tests | `nix run .#test` |
| Format Python | `uvx ruff format ... python tests scripts` |
| Python lint | `uvx ruff check python tests scripts` |
| Python tests | `pytest tests/python` |
| Mojo unit tests | `mojo run -I src src/tests/test_*.mojo` |
| Mojo parity diff | `python tests/parity/run_source_hash_parity.py` |
| All hygiene checks | `nix run .#check` |
| Ratchet pins | `nix run .#ratchet-lint` / `nix run .#ratchet-update` |
| Build mohaus wheel | `maturin build --release --out dist` |
| Build mohaus-mojo wheel | see "packaging the mojo parity ports" |
| Add Python dep | `mohaus add <spec>` |
| Add Mojo dep | `mohaus add --mojo <path>` |

## related plans

- `.claude/plans/V1.md` — v1 product contract.
- `.claude/plans/dogfood.md` — packaging cutover stages.
- `.claude/plans/mojo-migration.md` — per-crate Mojo port priority.
