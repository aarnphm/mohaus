# Known-answer tests for the pure-Mojo SHA256. Vectors come from FIPS 180-4
# Appendix B and NIST CAVP. If these break, the Rust differential will break
# too; this test localizes the failure.

from mohaus_hashing._sha256 import Sha256, sha256_hex_string
from std.testing import assert_equal


def test_empty_string() raises:
    assert_equal(
        sha256_hex_string(""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    )


def test_abc() raises:
    assert_equal(
        sha256_hex_string("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    )


def test_two_block_message() raises:
    var msg = "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
    assert_equal(
        sha256_hex_string(msg),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
    )


def test_streaming_matches_one_shot() raises:
    var hasher = Sha256.new()
    hasher.update_string("ab")
    hasher.update_string("c")
    assert_equal(
        hasher.hexdigest(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    )


def main() raises:
    test_empty_string()
    test_abc()
    test_two_block_message()
    test_streaming_matches_one_shot()
