# Parity tests for mohaus_toolchain. The corpus matches the Rust unit tests in
# `crates/mohaus-core/src/config.rs` and `crates/mohaus-core/src/toolchain.rs`.

from mohaus_toolchain import normalize_mojo_version_token
from std.testing import assert_equal


def test_normalize_cli_version() raises:
    assert_equal(normalize_mojo_version_token("Mojo 26.2.0 (abcd)"), "26.2.0")
    assert_equal(normalize_mojo_version_token("0.26.2.0"), "26.2.0")
    assert_equal(
        normalize_mojo_version_token("Mojo 1.0.0b2.dev2026050306 (dc0cf636)"),
        "1.0.0b2.dev2026050306",
    )


def test_normalize_preserves_nightly() raises:
    assert_equal(
        normalize_mojo_version_token("1.0.0b2.dev2026050306"),
        "1.0.0b2.dev2026050306",
    )


def main() raises:
    test_normalize_cli_version()
    test_normalize_preserves_nightly()
