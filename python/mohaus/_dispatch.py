"""Runtime dispatcher between the Rust orchestrator and the Mojo parity ports.

Default behavior: route every primitive through the bundled Rust pyo3
extension (`mohaus.mohaus_pep517`). When the optional `mohaus-mojo` package
is installed and a `mojo` toolchain is reachable, the dispatcher flips to the
Mojo `.mojopkg` implementations.

The flip can be forced off at any time with `MOHAUS_DISABLE_MOJO_PARITY=1`,
which keeps the Rust orchestrator on the hot path even when `mohaus-mojo` is
present. Useful for differential debugging when a Mojo regression is
suspected.
"""

from __future__ import annotations

import importlib
import os
from importlib.util import find_spec
from typing import TYPE_CHECKING

if TYPE_CHECKING:
  from types import ModuleType

_DISABLE_ENV = "MOHAUS_DISABLE_MOJO_PARITY"

_cached_decision: bool | None = None


def is_mojo_parity_active() -> bool:
  """Return True when mohaus should call into Mojo parity ports."""
  global _cached_decision
  if _cached_decision is not None:
    return _cached_decision
  _cached_decision = _decide()
  return _cached_decision


def reset_cache() -> None:
  """Reset the dispatch decision cache. Tests use this between scenarios."""
  global _cached_decision
  _cached_decision = None


def _decide() -> bool:
  if os.environ.get(_DISABLE_ENV) == "1":
    return False
  if find_spec("mohaus_mojo") is None:
    return False
  module = _load_mohaus_mojo()
  if module is None:
    return False
  try:
    return bool(module.is_runtime_available())
  except Exception:
    return False


def _load_mohaus_mojo() -> ModuleType | None:
  try:
    return importlib.import_module("mohaus_mojo")
  except Exception:
    return None


def active_backend_name() -> str:
  return "mojo" if is_mojo_parity_active() else "rust"
