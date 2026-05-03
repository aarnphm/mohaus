"""Editable-install rebuild hook used by mohaus-generated PEP 660 wheels."""

from __future__ import annotations

import os

from .mohaus_pep517 import rebuild_editable

_REBUILDING_ENV = "MOHAUS_EDITABLE_REBUILDING"


def ensure(project_root: str) -> None:
  if os.environ.get(_REBUILDING_ENV):
    return

  previous = os.environ.get(_REBUILDING_ENV)
  os.environ[_REBUILDING_ENV] = "1"
  try:
    rebuild_editable(project_root)
  finally:
    if previous is None:
      os.environ.pop(_REBUILDING_ENV, None)
    else:
      os.environ[_REBUILDING_ENV] = previous
