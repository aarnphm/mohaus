"""Editable-install rebuild hook used by mohaus-generated PEP 660 wheels."""

from __future__ import annotations

import os
import time
from pathlib import Path

from .mohaus_pep517 import rebuild_editable

_REBUILDING_ENV = "MOHAUS_EDITABLE_REBUILDING"
_HASH_DIR = ".mohaus"
_PROCESS_CACHE: dict[str, float] = {}
_DISABLE_SHORT_CIRCUIT_ENV = "MOHAUS_EDITABLE_FORCE"


def ensure(project_root: str) -> None:
  if os.environ.get(_REBUILDING_ENV):
    return
  if not os.environ.get(_DISABLE_SHORT_CIRCUIT_ENV) and _per_process_short_circuit(project_root):
    return

  previous = os.environ.get(_REBUILDING_ENV)
  os.environ[_REBUILDING_ENV] = "1"
  try:
    rebuild_editable(project_root)
    _PROCESS_CACHE[project_root] = _hash_dir_signature(project_root)
  finally:
    if previous is None:
      os.environ.pop(_REBUILDING_ENV, None)
    else:
      os.environ[_REBUILDING_ENV] = previous


def _per_process_short_circuit(project_root: str) -> bool:
  signature = _hash_dir_signature(project_root)
  if signature == 0.0:
    return False
  cached = _PROCESS_CACHE.get(project_root)
  if cached is None:
    return False
  return cached == signature


def _hash_dir_signature(project_root: str) -> float:
  hash_dir = Path(project_root) / _HASH_DIR
  if not hash_dir.is_dir():
    return 0.0
  latest = 0.0
  try:
    for entry in hash_dir.iterdir():
      if entry.suffix == ".hash":
        mtime = entry.stat().st_mtime
        if mtime > latest:
          latest = mtime
  except OSError:
    return 0.0
  if latest == 0.0:
    return 0.0
  return latest if latest <= time.time() else 0.0
