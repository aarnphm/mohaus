from __future__ import annotations

import os

import mohaus._editable as editable
import pytest


def test_editable_hook_skips_recursive_rebuild(monkeypatch: pytest.MonkeyPatch) -> None:
  monkeypatch.setenv("MOHAUS_EDITABLE_REBUILDING", "1")

  def fail_rebuild(_project_root: str) -> None:
    raise AssertionError("recursive rebuild should be skipped")

  monkeypatch.setattr(editable, "rebuild_editable", fail_rebuild)

  editable.ensure("/tmp/project")


def test_editable_hook_marks_child_processes(monkeypatch: pytest.MonkeyPatch) -> None:
  seen: list[str | None] = []

  def record_rebuild(_project_root: str) -> None:
    seen.append(os.environ.get("MOHAUS_EDITABLE_REBUILDING"))

  monkeypatch.delenv("MOHAUS_EDITABLE_REBUILDING", raising=False)
  monkeypatch.setattr(editable, "rebuild_editable", record_rebuild)

  editable.ensure("/tmp/project")

  assert seen == ["1"]
  assert "MOHAUS_EDITABLE_REBUILDING" not in os.environ
