"""Stage `.mojopkg` artifacts and scaffold templates into the mohaus-mojo
package source tree before `python -m build` runs.

CI invokes this after `mojo package src/<name> -o <name>.mojopkg` populates
`target/mojopkg/`. The script copies the four parity packages into
`packages/mohaus-mojo/src/mohaus_mojo/_mojopkg/` and mirrors the scaffold
templates into `packages/mohaus-mojo/src/mohaus_mojo/templates/`.

Templates have to live inside the package because Python wheels can't reach
out to the workspace root at install time. We keep the canonical templates
under `crates/mohaus-scaffold/src/templates/` and copy here so the bytes
stay byte-equal between the Rust and Mojo scaffold paths.
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

PARITY_PACKAGES = (
  "mohaus_toolchain",
  "mohaus_hashing",
  "mohaus_scaffold",
  "mohaus_stubgen",
)
TEMPLATE_FILES = (
  "pyproject.toml.tmpl",
  "lib.mojo.tmpl",
  "__init__.py.tmpl",
  "README.md.tmpl",
  "flake.nix.tmpl",
  "gitignore.tmpl",
  "gitattributes.tmpl",
  "LICENSE.tmpl",
)


def main(argv: list[str] | None = None) -> int:
  parser = argparse.ArgumentParser()
  parser.add_argument("--mojopkg-dir", type=Path, required=True, help="Where mojo package writes *.mojopkg")
  parser.add_argument(
    "--templates-source",
    type=Path,
    default=Path("crates/mohaus-scaffold/src/templates"),
    help="Canonical templates directory (Rust scaffold)",
  )
  parser.add_argument(
    "--package-root",
    type=Path,
    default=Path("python/mohaus_mojo"),
    help="mohaus-mojo package source tree (sibling of python/mohaus/)",
  )
  args = parser.parse_args(argv)

  package_root: Path = args.package_root
  mojopkg_dest = package_root / "_mojopkg"
  template_dest = package_root / "templates"

  mojopkg_dest.mkdir(parents=True, exist_ok=True)
  template_dest.mkdir(parents=True, exist_ok=True)

  failures: list[str] = []
  for name in PARITY_PACKAGES:
    source = args.mojopkg_dir / f"{name}.mojopkg"
    if not source.is_file():
      failures.append(f"missing {source}")
      continue
    shutil.copy2(source, mojopkg_dest / source.name)

  for name in TEMPLATE_FILES:
    source = args.templates_source / name
    if not source.is_file():
      failures.append(f"missing template {source}")
      continue
    shutil.copy2(source, template_dest / name)

  if failures:
    for line in failures:
      print(f"error: {line}", file=sys.stderr)
    return 1

  for stale in (mojopkg_dest / ".gitkeep", template_dest / ".gitkeep"):
    stale.unlink(missing_ok=True)
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
