"""Console-script adapter for the Rust mohaus CLI."""

from __future__ import annotations

import importlib.metadata as metadata
import json
import os
import sys
from pathlib import Path
from urllib.parse import unquote, urlparse

from .mohaus_pep517 import cli

_SELF_FIND_LINKS_ENV = "MOHAUS_SELF_FIND_LINKS"
_SELF_WHEEL_ENV = "MOHAUS_SELF_WHEEL"


def main() -> None:
  _set_self_find_links()
  raise SystemExit(cli(sys.argv[1:]))


def _set_self_find_links() -> None:
  if os.environ.get(_SELF_FIND_LINKS_ENV):
    return

  try:
    distribution = metadata.distribution("mohaus")
  except metadata.PackageNotFoundError:
    return

  text = distribution.read_text("direct_url.json")

  wheel = _wheel_from_direct_url_text(text)
  if wheel is not None:
    os.environ.setdefault(_SELF_WHEEL_ENV, str(wheel))
    os.environ.setdefault(_SELF_FIND_LINKS_ENV, str(wheel.parent))
    return

  wheelhouse = _wheelhouse_from_direct_url_text(text)
  if wheelhouse is not None:
    os.environ.setdefault(_SELF_FIND_LINKS_ENV, wheelhouse)


def _wheelhouse_from_direct_url_text(text: str | None) -> str | None:
  wheel = _wheel_from_direct_url_text(text)
  if wheel is not None:
    return str(wheel.parent)

  project = _editable_project_root_from_direct_url_text(text)
  if project is None:
    return None
  wheels = project / "target" / "wheels"
  if not wheels.is_dir():
    return None
  if not any(p.suffix == ".whl" and p.is_file() for p in wheels.iterdir()):
    return None
  return str(wheels)


def _wheel_from_direct_url_text(text: str | None) -> Path | None:
  parsed = _parse_direct_url_text(text)
  if parsed is None:
    return None
  raw, path = parsed

  dir_info = raw.get("dir_info")
  if isinstance(dir_info, dict) and dir_info.get("editable"):
    return None

  if path.suffix != ".whl" or not path.is_file():
    return None
  return path


def _editable_project_root_from_direct_url_text(text: str | None) -> Path | None:
  parsed = _parse_direct_url_text(text)
  if parsed is None:
    return None
  raw, path = parsed
  dir_info = raw.get("dir_info")
  if not isinstance(dir_info, dict) or not dir_info.get("editable"):
    return None
  if not path.is_dir():
    return None
  return path


def _parse_direct_url_text(text: str | None) -> tuple[dict[str, object], Path] | None:
  if text is None:
    return None
  try:
    raw: object = json.loads(text)
  except json.JSONDecodeError:
    return None
  if not isinstance(raw, dict):
    return None
  url = raw.get("url")
  if not isinstance(url, str):
    return None
  parsed = urlparse(url)
  if parsed.scheme != "file":
    return None
  return raw, Path(unquote(parsed.path))
