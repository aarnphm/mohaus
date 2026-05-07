"""Cross-check the Rust and Mojo `source_hash` implementations.

Hashes every fixture under `tests/fixtures/` with both the Rust crate (via
the in-process `mohaus._mohaus_pep517` helper exposed here) and the Mojo
package (`src/mohaus_hashing/`) by shelling out to `mojo run`. Asserts
byte-for-byte identical hex digests.

CI invokes this script after `mojo package` succeeds for the parity ports;
local runs need both `mojo` on PATH and `mohaus` installed in the active
environment.
"""

from __future__ import annotations

import os
import shlex
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURES = REPO_ROOT / "tests" / "fixtures"
MOJO_RUNNER = REPO_ROOT / "tests" / "parity" / "_mojo_hash_runner.mojo"
MOJO_RUN_FLAGS_ENV = "MOHAUS_MOJO_RUN_FLAGS"


def _rust_tree_hash(src_dir: Path) -> str:
  from mohaus.mohaus_pep517 import tree_hash_for_dir  # type: ignore[attr-defined]

  return tree_hash_for_dir(str(src_dir))


def _mojo_tree_hash(project_dir: Path) -> str:
  mojo_run_flags = shlex.split(os.environ.get(MOJO_RUN_FLAGS_ENV, ""))
  result = subprocess.run(
    ["mojo", "run", *mojo_run_flags, "-I", str(REPO_ROOT / "src"), str(MOJO_RUNNER), str(project_dir)],
    check=True,
    capture_output=True,
    text=True,
  )
  return result.stdout.strip().splitlines()[-1]


def _has_rust_helper() -> bool:
  try:
    from mohaus.mohaus_pep517 import tree_hash_for_dir  # noqa: F401
  except ImportError:
    return False
  return True


def main() -> int:
  if not _has_rust_helper():
    print(
      "skipping: mohaus.mohaus_pep517 does not export tree_hash_for_dir",
      file=sys.stderr,
    )
    return 0

  failures: list[str] = []
  for fixture in sorted(FIXTURES.iterdir()):
    if not (fixture / "pyproject.toml").is_file():
      continue
    src_dir = fixture / "src"
    if not src_dir.is_dir():
      continue
    rust = _rust_tree_hash(src_dir)
    mojo = _mojo_tree_hash(fixture)
    if rust == mojo:
      print(f"OK  {fixture.name}: {rust}")
    else:
      failures.append(f"{fixture.name}: rust={rust} mojo={mojo}")

  if failures:
    for line in failures:
      print(f"FAIL {line}", file=sys.stderr)
    return 1
  return 0


if __name__ == "__main__":
  raise SystemExit(main())
