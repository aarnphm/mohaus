# Smoke test that exercises the Mojo scaffold port end-to-end. Differential
# coverage with the Rust scaffold lives in `crates/mohaus-scaffold/src/lib.rs`
# unit tests; this test only proves the Mojo path renders the templates and
# round-trips into a parseable project.

from mohaus_scaffold import DEFAULT_MOJO_VERSION, ScaffoldOptions, scaffold_project
from std.os import getenv
from std.os.path import isfile
from std.pathlib import Path
from std.testing import assert_equal, assert_true


def _scratch_dir(suffix: String) -> Path:
    var base = getenv("TMPDIR", "/tmp")
    return Path(base).joinpath(suffix)


def test_scaffold_writes_expected_files() raises:
    var destination = _scratch_dir("mohaus-scaffold-acme")
    var templates_dir = String(Path("src").joinpath("mohaus_scaffold").joinpath("templates"))
    scaffold_project(ScaffoldOptions("acme", String(destination), templates_dir))
    assert_true(isfile(String(destination.joinpath("pyproject.toml"))))
    assert_true(isfile(String(destination.joinpath("src").joinpath("lib.mojo"))))
    assert_true(isfile(String(destination.joinpath("python").joinpath("acme").joinpath("__init__.py"))))
    assert_true(isfile(String(destination.joinpath("LICENSE"))))
    assert_true(isfile(String(destination.joinpath(".mojo-version"))))
    assert_equal(destination.joinpath(".mojo-version").read_text(), DEFAULT_MOJO_VERSION)
    var pyproject = destination.joinpath("pyproject.toml").read_text()
    assert_true(len(pyproject.split("mojo==" + DEFAULT_MOJO_VERSION)) > 1)


def main() raises:
    test_scaffold_writes_expected_files()
