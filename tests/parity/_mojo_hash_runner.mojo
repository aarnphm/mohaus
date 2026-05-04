# Helper invoked by tests/parity/run_source_hash_parity.py. Reads a project
# root from argv, hashes its `src/` tree (matching the canonical mohaus
# layout), and prints the hex digest on the last line of stdout.

from std.sys import argv

from mohaus_hashing import source_hash_for_dir
from std.pathlib import Path


def main() raises:
    var args = argv()
    if len(args) < 2:
        raise Error("usage: _mojo_hash_runner.mojo <project_dir>")
    var project_dir = String(args[1])
    var src_root = Path(project_dir).joinpath("src")
    var digest = source_hash_for_dir(String(src_root))
    print(digest)
