# Smoke test that exercises the Mojo scaffold port end-to-end. Differential
# coverage with the Rust scaffold lives in `crates/mohaus-scaffold/src/lib.rs`
# unit tests; this test only proves the Mojo path renders the templates and
# round-trips into a parseable project.

from mohaus_scaffold import ScaffoldOptions, scaffold_project
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
    assert_true(isfile(String(destination.joinpath("flake.nix"))))
    assert_true(isfile(String(destination.joinpath("src").joinpath("lib.mojo"))))
    assert_true(isfile(String(destination.joinpath("python").joinpath("acme").joinpath("__init__.py"))))
    assert_true(isfile(String(destination.joinpath("LICENSE"))))
    assert_true(isfile(String(destination.joinpath(".gitattributes"))))
    assert_true(not isfile(String(destination.joinpath(".mojo-version"))))
    var pyproject = destination.joinpath("pyproject.toml").read_text()
    assert_true(len(pyproject.split('"modular"')) > 1)
    assert_true(len(pyproject.split('"mojo==')) == 1)
    assert_true(len(pyproject.split('"mojo-compiler==')) == 1)
    assert_true(len(pyproject.split('"mojo-compiler-mojo-libs==')) == 1)
    assert_true(len(pyproject.split('"mojo-lldb-libs==')) == 1)
    assert_true(
        len(
            pyproject.split(
                '[tool.uv]\nextra-index-url = [\n  "https://aarnphm.github.io/mohaus/simple",\n '
                ' "https://whl.modular.com/nightly/simple/",\n]\nprerelease = "allow"'
            )
        )
        > 1
    )
    assert_true(len(pyproject.split('extend-include = ["*.ipynb"]')) > 1)
    assert_true(len(pyproject.split('[tool.ty.rules]\nall = "error"')) > 1)
    var flake = destination.joinpath("flake.nix").read_text()
    assert_true(len(flake.split('description = "acme: mixed Python and Mojo package scaffolded by mohaus";')) > 1)
    assert_true(len(flake.split('git-hooks-nix.url = "github:cachix/git-hooks.nix";')) > 1)
    assert_true(len(flake.split('mohaus.url = "github:aarnphm/mohaus";')) > 1)
    assert_true(len(flake.split("mohaus develop")) > 1)
    assert_true(len(flake.split("pre-commit = git-hooks-nix.lib.${system}.run")) > 1)
    assert_true(len(flake.split("uvx ty check")) > 1)
    assert_true(len(flake.split("oxfmt")) == 1)
    var readme = destination.joinpath("README.md").read_text()
    assert_true(len(readme.split("https://whl.modular.com/nightly/simple/")) > 1)
    var gitignore = destination.joinpath(".gitignore").read_text()
    assert_true(len(gitignore.split("/vendor/")) > 1)
    assert_true(len(gitignore.split("/benches/")) == 1)
    var gitattributes = destination.joinpath(".gitattributes").read_text()
    assert_true(len(gitattributes.split("/vendor/** linguist-vendored")) > 1)


def main() raises:
    test_scaffold_writes_expected_files()
