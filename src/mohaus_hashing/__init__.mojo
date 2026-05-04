# mohaus-hashing (Mojo parity port)
#
# Mirrors `mohaus_core::editable::source_hash`. Native pure-Mojo SHA256 lives
# in `_sha256.mojo`; the walker uses `os.listdir` + `pathlib.Path` so we never
# touch CPython.
#
# Byte sequence fed into the hasher matches the Rust crate exactly:
#   <module name bytes>
#   <module entry path bytes>
#   for each flag:
#     <flag bytes> 0x00
#   for each file under mojo-src + each include path (sorted lex by relpath):
#     <relative path bytes> 0x00 <file contents bytes>
#
# Differential parity is checked in `src/tests/test_hashing.mojo` and the
# Rust integration test under `crates/mohaus-core/tests/`.

from std.collections import List
from std.os import listdir
from std.os.path import isdir, isfile
from std.pathlib import Path

from ._sha256 import Sha256


def _is_relevant_extension(name: String) -> Bool:
    return name.endswith(".mojo") or name.endswith(".🔥") or name.endswith(".mojopkg")


def _walk_relevant_files(root: Path, mut out: List[Path]) raises:
    var stack = List[Path]()
    stack.append(root)
    while len(stack) > 0:
        var current = stack.pop()
        for entry_name in listdir(String(current)):
            var entry = current.joinpath(String(entry_name))
            var entry_str = String(entry)
            if isdir(entry_str):
                stack.append(entry)
            elif isfile(entry_str) and _is_relevant_extension(String(entry_name)):
                out.append(entry)


def _sort_paths(mut paths: List[Path]):
    var n = len(paths)
    for i in range(1, n):
        var j = i
        while j > 0 and String(paths[j - 1]) > String(paths[j]):
            var tmp = paths[j - 1]
            paths[j - 1] = paths[j]
            paths[j] = tmp
            j -= 1


def _relative_to(path: Path, root: Path) -> String:
    var path_str = String(path)
    var root_str = String(root)
    if not path_str.startswith(root_str):
        return path_str
    var path_bytes = path_str.as_bytes()
    var prefix_len = root_str.byte_length()
    var start = prefix_len
    var slash = UInt8(47)
    if start < len(path_bytes) and path_bytes[start] == slash:
        start += 1
    var trimmed = List[UInt8]()
    for i in range(start, len(path_bytes)):
        trimmed.append(path_bytes[i])
    trimmed.append(UInt8(0))
    return String(unsafe_from_utf8_ptr=trimmed.unsafe_ptr())


def _hash_tree_into(mut hasher: Sha256, root: Path) raises:
    if not isdir(String(root)):
        return
    var files = List[Path]()
    _walk_relevant_files(root, files)
    _sort_paths(files)
    var nul = List[UInt8]()
    nul.append(UInt8(0))
    for index in range(len(files)):
        var path_obj = files[index]
        var relative = _relative_to(path_obj, root)
        hasher.update_string(relative)
        hasher.update_bytes(nul)
        hasher.update_bytes(path_obj.read_bytes())


def source_hash_for_dir(root: String) raises -> String:
    """Hash every `.mojo` / `.🔥` / `.mojopkg` file beneath `root`.

    Output matches the Rust crate's `source_hash` for the same input when no
    module name, entry, flags, or include paths are supplied.
    """
    var hasher = Sha256.new()
    _hash_tree_into(hasher, Path(root))
    return hasher.hexdigest()


def source_hash_for_module(
    project_dir: String,
    module_name: String,
    module_entry: String,
    mojo_src: String,
    mojo_flags: List[String],
    mojo_include_paths: List[String],
) raises -> String:
    """Full parity surface mirroring `mohaus_core::editable::source_hash`."""
    var hasher = Sha256.new()
    hasher.update_string(module_name)
    hasher.update_string(module_entry)
    var nul = List[UInt8]()
    nul.append(UInt8(0))
    for flag in mojo_flags:
        hasher.update_string(String(flag))
        hasher.update_bytes(nul)

    var project_path = Path(project_dir)
    _hash_tree_into(hasher, project_path.joinpath(mojo_src))
    for include in mojo_include_paths:
        var include_path = project_path.joinpath(String(include))
        if isdir(String(include_path)):
            _hash_tree_into(hasher, include_path)
        elif isfile(String(include_path)):
            hasher.update_string(String(include_path.name()))
            hasher.update_bytes(nul)
            hasher.update_bytes(include_path.read_bytes())

    return hasher.hexdigest()
