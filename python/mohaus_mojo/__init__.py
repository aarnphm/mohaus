"""Mojo parity ports of mohaus build primitives.

`_mojopkg/` and `templates/` are NOT checked into git. CI runs
`mojo package src/<name> -o <name>.mojopkg` for each parity port, then
`scripts/stage_mohaus_mojo_assets.py` copies the compiled `.mojopkg` files
into `_mojopkg/` and the canonical scaffold templates from
`crates/mohaus-scaffold/src/templates/` into `templates/`. The wheel built
by `mohaus build` (run from `python/mohaus_mojo/`) bundles those staged
artifacts.

In a fresh development checkout neither directory exists, and
`is_runtime_available()` returns `False`. That's the correct fallback: the
mohaus dispatcher stays on the Rust orchestrator until a real `mohaus-mojo`
wheel is installed.

Public API:
    has_mojopkg(name) -> bool
    mojopkg_path(name) -> pathlib.Path
    templates_dir() -> pathlib.Path
    is_runtime_available() -> bool
"""

from __future__ import annotations

import os
import shutil
from pathlib import Path

__all__ = [
  "has_mojopkg",
  "is_runtime_available",
  "mojopkg_path",
  "templates_dir",
]

_PACKAGE_ROOT = Path(__file__).resolve().parent
_MOJOPKG_ROOT = _PACKAGE_ROOT / "_mojopkg"
_TEMPLATES_ROOT = _PACKAGE_ROOT / "templates"
_KNOWN = ("mohaus_toolchain", "mohaus_hashing", "mohaus_scaffold")


def has_mojopkg(name: str) -> bool:
  return (_MOJOPKG_ROOT / f"{name}.mojopkg").is_file()


def mojopkg_path(name: str) -> Path:
  candidate = _MOJOPKG_ROOT / f"{name}.mojopkg"
  if not candidate.is_file():
    raise FileNotFoundError(f"mohaus-mojo does not bundle {name}.mojopkg; available: {_KNOWN}")
  return candidate


def templates_dir() -> Path:
  if not _TEMPLATES_ROOT.is_dir():
    raise FileNotFoundError(f"mohaus-mojo templates directory missing: {_TEMPLATES_ROOT}")
  return _TEMPLATES_ROOT


def is_runtime_available() -> bool:
  """True when host has `mojo` plus all three parity packages bundled."""
  if os.environ.get("MOHAUS_MOJO"):
    if Path(os.environ["MOHAUS_MOJO"]).is_file():
      return _all_packages_present()
  if shutil.which("mojo") is not None:
    return _all_packages_present()
  if modular_home := os.environ.get("MODULAR_HOME"):
    if (Path(modular_home) / "bin" / "mojo").is_file():
      return _all_packages_present()
  return False


def _all_packages_present() -> bool:
  return all(has_mojopkg(name) for name in _KNOWN)
