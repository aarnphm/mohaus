"""Editable-install rebuild hook used by mohaus-generated PEP 660 wheels."""

from __future__ import annotations

from .mohaus_pep517 import rebuild_editable


def ensure(project_root: str) -> None:
  rebuild_editable(project_root)
