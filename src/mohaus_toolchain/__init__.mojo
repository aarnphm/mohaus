# mohaus-toolchain (Mojo parity port)
#
# Mirrors `crates/mohaus-core/src/toolchain.rs`. Resolves the Mojo executable
# from MOHAUS_MOJO, PATH, MODULAR_HOME/bin/mojo and probes its version. The
# mojo CLI's `--version` output is parsed with the same normalization rule as
# the Rust crate (`normalize_mojo_version_token`).
#
# This module deliberately keeps a 1:1 surface with the Rust API so the parity
# diff stays small. When the cutover happens, `mohaus-pep517` calls into this
# module instead of `mohaus-core::toolchain`.

from std.collections import List
from std.os import getenv
from std.os.path import exists, realpath
from std.pathlib import Path
from std.subprocess import run as subprocess_run

comptime MOHAUS_MOJO_ENV = "MOHAUS_MOJO"
comptime MODULAR_HOME_ENV = "MODULAR_HOME"
comptime PATH_ENV = "PATH"


@fieldwise_init
struct MojoToolchain(Movable):
    """Resolved Mojo executable paired with its version line."""

    var executable: String
    var version_output: String


def resolve_mojo_executable() raises -> String:
    """Walk $MOHAUS_MOJO, $PATH, $MODULAR_HOME/bin/mojo and return the first hit.

    Raises:
        Error if no executable is found.
    """
    var candidates = _mojo_candidates()
    if len(candidates) > 0:
        return candidates[0]

    raise Error("could not find a Mojo executable; searched $MOHAUS_MOJO, $PATH, and $MODULAR_HOME/bin/mojo")


def _mojo_override_candidate() -> String:
    var override_path = getenv(MOHAUS_MOJO_ENV)
    if override_path.byte_length() > 0 and exists(override_path):
        return _normalize_mojo_candidate(override_path)
    return String("")


def _mojo_candidates() -> List[String]:
    var candidates = List[String]()
    var override_path = _mojo_override_candidate()
    if override_path.byte_length() > 0:
        _push_unique_candidate(candidates, override_path)
    var rest = _path_and_modular_home_candidates()
    for candidate in rest:
        _push_unique_candidate(candidates, String(candidate))
    return candidates


def _path_and_modular_home_candidates() -> List[String]:
    var candidates = List[String]()

    var path_value = getenv(PATH_ENV)
    if path_value.byte_length() > 0:
        var separator = ":"
        for entry in path_value.split(separator):
            if entry.byte_length() == 0:
                continue
            var candidate = String(entry) + "/mojo"
            if exists(candidate):
                _push_unique_candidate(candidates, _normalize_mojo_candidate(candidate))

    var modular_home = getenv(MODULAR_HOME_ENV)
    if modular_home.byte_length() > 0:
        var candidate = modular_home + "/bin/mojo"
        if exists(candidate):
            _push_unique_candidate(candidates, _normalize_mojo_candidate(candidate))

    return candidates


def _normalize_mojo_candidate(candidate: String) -> String:
    try:
        return realpath(candidate)
    except:
        return candidate


def _push_unique_candidate(mut candidates: List[String], candidate: String):
    for existing in candidates:
        if String(existing) == candidate:
            return
    candidates.append(candidate)


def probe_mojo_version(executable: String) raises -> String:
    """Run `<executable> --version` and return trimmed stdout."""
    var output = subprocess_run(executable + " --version")
    return String(String(output).strip())


def normalize_mojo_version_token(value: String) -> String:
    """Mirror of `mohaus_core::config::normalize_mojo_version_token`.

    The output must match the Rust implementation byte-for-byte; the parity
    test under `mojo/tests/test_toolchain.mojo` pins a fixture corpus.
    """
    var parts = value.split()
    var token = String("")
    for piece in parts:
        var s = String(piece)
        var bytes = s.as_bytes()
        var has_digit = False
        for i in range(len(bytes)):
            var b = bytes[i]
            if b >= UInt8(48) and b <= UInt8(57):
                has_digit = True
                break
        if has_digit:
            token = s
            break
    if token.byte_length() == 0:
        token = value

    var token_bytes = token.as_bytes()
    var start = 0
    while start < len(token_bytes) and not _is_alnum(token_bytes[start]):
        start += 1
    var end = len(token_bytes)
    while end > start and not _is_alnum(token_bytes[end - 1]):
        end -= 1
    if start < len(token_bytes) and token_bytes[start] == UInt8(118):
        start += 1
    var slice_bytes = List[UInt8]()
    for i in range(start, end):
        slice_bytes.append(token_bytes[i])
    slice_bytes.append(UInt8(0))
    var trimmed = String(unsafe_from_utf8_ptr=slice_bytes.unsafe_ptr())

    var dots = trimmed.split(".")
    if len(dots) >= 4 and String(dots[0]) == "0":
        var rest = List[String]()
        for i in range(1, len(dots)):
            rest.append(String(dots[i]))
        return ".".join(rest)
    return trimmed


def resolve_verified_mojo(expected: String) raises -> MojoToolchain:
    """Resolve + probe + match. Raises with the same shape the Rust crate emits."""
    var expected_normalized = normalize_mojo_version_token(expected)
    var override_path = _mojo_override_candidate()
    if override_path.byte_length() > 0:
        return _verify_mojo_candidate(override_path, expected_normalized)
    return _resolve_verified_mojo_from_candidates(expected_normalized, _path_and_modular_home_candidates())


def _resolve_verified_mojo_from_candidates(expected: String, candidates: List[String]) raises -> MojoToolchain:
    var first_executable = String("")
    var first_actual = String("")
    for candidate in candidates:
        var executable = String(candidate)
        var version_output = probe_mojo_version(executable)
        var actual = normalize_mojo_version_token(version_output)
        if actual == expected:
            return MojoToolchain(executable, version_output)
        if first_executable.byte_length() == 0:
            first_executable = executable
            first_actual = actual
    if first_executable.byte_length() > 0:
        _raise_mojo_version_mismatch(expected, first_executable, first_actual)
    raise Error("could not find a Mojo executable; searched $MOHAUS_MOJO, $PATH, and $MODULAR_HOME/bin/mojo")


def _verify_mojo_candidate(executable: String, expected: String) raises -> MojoToolchain:
    var version_output = probe_mojo_version(executable)
    var actual = normalize_mojo_version_token(version_output)
    if actual != expected:
        _raise_mojo_version_mismatch(expected, executable, actual)
    return MojoToolchain(executable, version_output)


def _raise_mojo_version_mismatch(expected: String, executable: String, actual: String) raises:
    raise Error(
        "Mojo version mismatch: project pins `",
        expected,
        "`, but `",
        executable,
        "` reported `",
        actual,
        "`",
    )


def _is_alnum(byte: UInt8) -> Bool:
    return (
        (byte >= UInt8(48) and byte <= UInt8(57))
        or (byte >= UInt8(97) and byte <= UInt8(122))
        or (byte >= UInt8(65) and byte <= UInt8(90))
    )
