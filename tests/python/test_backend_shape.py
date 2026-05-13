from __future__ import annotations

import inspect
import tomllib
from pathlib import Path

import mohaus.backend as backend


def test_backend_exports_pep_hooks() -> None:
  for name in [
    "build_wheel",
    "build_sdist",
    "build_editable",
    "get_requires_for_build_wheel",
    "get_requires_for_build_editable",
    "prepare_metadata_for_build_editable",
    "prepare_metadata_for_build_wheel",
  ]:
    value = getattr(backend, name)
    assert callable(value)


def test_build_wheel_signature_starts_with_wheel_directory() -> None:
  signature = inspect.signature(backend.build_wheel)
  assert next(iter(signature.parameters)) == "wheel_directory"


def test_project_readme_path_exists() -> None:
  root = Path(__file__).resolve().parents[2]
  pyproject = tomllib.loads((root / "pyproject.toml").read_text())
  readme = pyproject["project"]["readme"]

  assert isinstance(readme, str)
  assert (root / readme).is_file()
