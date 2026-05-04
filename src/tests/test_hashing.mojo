# Parity smoke for mohaus_hashing.source_hash_for_dir. Native pure-Mojo SHA256
# now powers the hash; CI runs `tests/fixtures/with_include_paths` through both
# this Mojo path and the Rust crate and compares hex digests byte-for-byte.

from mohaus_hashing import source_hash_for_dir
from std.os import getenv, makedirs
from std.pathlib import Path
from std.testing import assert_true


def _scratch_dir(suffix: String) -> Path:
    var base = getenv("TMPDIR", "/tmp")
    return Path(base).joinpath(suffix)


def test_hash_changes_when_source_changes() raises:
    var root = _scratch_dir("mohaus_mojo_hash_test")
    makedirs(String(root), exist_ok=True)
    var src = root.joinpath("src")
    makedirs(String(src), exist_ok=True)
    var first_path = src.joinpath("lib.mojo")
    var first_handle = open(String(first_path), "w")
    first_handle.write("def main(): pass\n")
    first_handle.close()

    var first = source_hash_for_dir(String(root))

    var second_handle = open(String(first_path), "w")
    second_handle.write("def main(): print(1)\n")
    second_handle.close()

    var second = source_hash_for_dir(String(root))
    assert_true(first != second, "hash must change when source changes")


def test_hash_is_deterministic() raises:
    var root = _scratch_dir("mohaus_mojo_hash_determinism")
    makedirs(String(root), exist_ok=True)
    var src = root.joinpath("src")
    makedirs(String(src), exist_ok=True)
    var p = src.joinpath("lib.mojo")
    var handle = open(String(p), "w")
    handle.write("def main(): pass\n")
    handle.close()

    var first = source_hash_for_dir(String(root))
    var second = source_hash_for_dir(String(root))
    assert_true(first == second, "hash must be deterministic across calls")


def main() raises:
    test_hash_changes_when_source_changes()
    test_hash_is_deterministic()
