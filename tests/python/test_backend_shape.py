from __future__ import annotations

import inspect

import mohaus.backend as backend


def test_backend_exports_pep_hooks() -> None:
  for name in [
    "build_wheel",
    "build_sdist",
    "build_editable",
    "get_requires_for_build_wheel",
    "get_requires_for_build_editable",
    "prepare_metadata_for_build_wheel",
  ]:
    value = getattr(backend, name)
    assert callable(value)


def test_build_wheel_signature_starts_with_wheel_directory() -> None:
  signature = inspect.signature(backend.build_wheel)
  assert next(iter(signature.parameters)) == "wheel_directory"
