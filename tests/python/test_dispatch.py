from __future__ import annotations

import importlib
from types import SimpleNamespace

import mohaus._dispatch as dispatch
import pytest


@pytest.fixture(autouse=True)
def _reset_cache() -> None:
  dispatch.reset_cache()
  yield
  dispatch.reset_cache()


def test_default_backend_is_rust(monkeypatch: pytest.MonkeyPatch) -> None:
  monkeypatch.delenv("MOHAUS_DISABLE_MOJO_PARITY", raising=False)
  monkeypatch.setattr(dispatch, "find_spec", lambda _name: None)
  assert dispatch.active_backend_name() == "rust"
  assert dispatch.is_mojo_parity_active() is False


def test_mojo_backend_active_when_runtime_available(monkeypatch: pytest.MonkeyPatch) -> None:
  monkeypatch.delenv("MOHAUS_DISABLE_MOJO_PARITY", raising=False)
  monkeypatch.setattr(dispatch, "find_spec", lambda _name: object())
  fake = SimpleNamespace(is_runtime_available=lambda: True)
  monkeypatch.setattr(dispatch, "_load_mohaus_mojo", lambda: fake)
  assert dispatch.is_mojo_parity_active() is True
  assert dispatch.active_backend_name() == "mojo"


def test_force_disable_env_keeps_rust(monkeypatch: pytest.MonkeyPatch) -> None:
  monkeypatch.setenv("MOHAUS_DISABLE_MOJO_PARITY", "1")
  monkeypatch.setattr(dispatch, "find_spec", lambda _name: object())
  fake = SimpleNamespace(is_runtime_available=lambda: True)
  monkeypatch.setattr(dispatch, "_load_mohaus_mojo", lambda: fake)
  assert dispatch.is_mojo_parity_active() is False


def test_runtime_unavailable_falls_back_to_rust(monkeypatch: pytest.MonkeyPatch) -> None:
  monkeypatch.delenv("MOHAUS_DISABLE_MOJO_PARITY", raising=False)
  monkeypatch.setattr(dispatch, "find_spec", lambda _name: object())
  fake = SimpleNamespace(is_runtime_available=lambda: False)
  monkeypatch.setattr(dispatch, "_load_mohaus_mojo", lambda: fake)
  assert dispatch.is_mojo_parity_active() is False


def test_decision_is_cached(monkeypatch: pytest.MonkeyPatch) -> None:
  calls = {"count": 0}

  def fake_decide() -> bool:
    calls["count"] += 1
    return False

  monkeypatch.setattr(dispatch, "_decide", fake_decide)
  dispatch.is_mojo_parity_active()
  dispatch.is_mojo_parity_active()
  assert calls["count"] == 1


def test_module_imports_clean() -> None:
  importlib.reload(dispatch)
  dispatch.reset_cache()
