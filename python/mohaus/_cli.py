"""Console-script adapter for the Rust mohaus CLI."""

from __future__ import annotations

import sys

from .mohaus_pep517 import cli


def main() -> None:
  raise SystemExit(cli(sys.argv[1:]))
