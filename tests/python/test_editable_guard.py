from __future__ import annotations

import os
from pathlib import Path

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


def test_editable_hook_short_circuits_when_signature_unchanged(
  monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
  hash_dir = tmp_path / ".mohaus"
  hash_dir.mkdir()
  (hash_dir / "demo.hash").write_text("abc")

  call_count = {"value": 0}

  def record_rebuild(_project_root: str) -> None:
    call_count["value"] += 1

  monkeypatch.delenv("MOHAUS_EDITABLE_REBUILDING", raising=False)
  monkeypatch.delenv("MOHAUS_EDITABLE_FORCE", raising=False)
  monkeypatch.setattr(editable, "_PROCESS_CACHE", {}, raising=False)
  monkeypatch.setattr(editable, "rebuild_editable", record_rebuild)

  editable.ensure(str(tmp_path))
  editable.ensure(str(tmp_path))

  assert call_count["value"] == 1


def test_editable_hook_force_env_disables_short_circuit(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
  hash_dir = tmp_path / ".mohaus"
  hash_dir.mkdir()
  (hash_dir / "demo.hash").write_text("abc")

  call_count = {"value": 0}

  def record_rebuild(_project_root: str) -> None:
    call_count["value"] += 1

  monkeypatch.delenv("MOHAUS_EDITABLE_REBUILDING", raising=False)
  monkeypatch.setenv("MOHAUS_EDITABLE_FORCE", "1")
  monkeypatch.setattr(editable, "_PROCESS_CACHE", {}, raising=False)
  monkeypatch.setattr(editable, "rebuild_editable", record_rebuild)

  editable.ensure(str(tmp_path))
  editable.ensure(str(tmp_path))

  assert call_count["value"] == 2
