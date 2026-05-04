# mohaus

This is the canonical operating policy for this repository.

## priorities

- Keep mohaus maturin-shaped: PEP 517/660 backend, Rust CLI, mixed `python/`
  layout, editable installs, wheels, sdists, and uv-friendly publishing.
- Treat `.claude/plans/V1.md` as the v1 product contract.
- Prefer repo-native commands. Use `nix develop` for interactive work and
  `nix run .#check` for the full local hygiene pass before raw `cargo`, `ruff`,
  or ad hoc shell.
- `nix develop` installs pre-commit hooks from the flake. Do not hand-edit the
  generated `.pre-commit-config.yaml`.
- `nix develop` also creates `.venv` and installs `mohaus` editable with dev
  extras through nix-provided `maturin`, so `mohaus`, `pytest`, and the local
  backend are immediately runnable.
- `nix develop` generates shell completions from that editable `mohaus` and
  exposes them through `.venv/share` for bash, fish, and zsh.
- Official docs override stale local skills when Mojo syntax or packaging
  behavior changed.
- Do not run git commands that mutate state. Read-only commands such as
  `git status`, `git diff`, and `git show` are allowed.
- Assume other agents or the human may edit concurrently. Refresh context before
  summarizing or touching files.

## code hygiene

- No breadcrumbs. If code moves or dies, remove it cleanly.
- Prefer first-principles fixes over adapters that hide the broken ownership
  path.
- Do not use `unwrap`, `expect`, `panic!`, `todo!`, or `unimplemented!` in
  production Rust paths.
- Do not use `any` or loose dict-shaped Python APIs. Model real shapes.
- Use strong typed boundaries for package names, module names, Mojo versions,
  source roots, and wheel tags.
- Keep global mutable state out of core logic.
- Add dependencies only when the maintenance surface is worth it. Prefer
  established crates and document the reason when the dependency is not obvious.

## testing

- Prefer unit, integration, and e2e tests over mocks.
- Run the narrowest tests that cover your edits, then expand when touching
  shared behavior or packaging contracts.
- Rust changes should pass:
  `nix run .#fmt`
  `nix run .#clippy`
  `nix run .#test`
- Python formatting uses:
  `uvx ruff format --config "indent-width=2" --config "line-length=119" --config "preview=true"`
- Pre-commit hooks are flake-owned. Use
  `nix develop -c pre-commit run --all-files` to run them manually.
- Mojo/Python packaging changes should include at least one wheel or editable
  path check when Mojo is available.
