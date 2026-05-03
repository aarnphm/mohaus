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

  wheel = _wheel_from_direct_url_text(distribution.read_text("direct_url.json"))
  if wheel is not None:
    os.environ.setdefault(_SELF_WHEEL_ENV, str(wheel))
    os.environ.setdefault(_SELF_FIND_LINKS_ENV, str(wheel.parent))


def _wheelhouse_from_direct_url_text(text: str | None) -> str | None:
  wheel = _wheel_from_direct_url_text(text)
  if wheel is None:
    return None
  return str(wheel.parent)


def _wheel_from_direct_url_text(text: str | None) -> Path | None:
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

  wheel = Path(unquote(parsed.path))
  if wheel.suffix != ".whl" or not wheel.is_file():
    return None

  return wheel
