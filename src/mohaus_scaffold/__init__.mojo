# mohaus-scaffold (Mojo parity port)
#
# Mirrors `crates/mohaus-scaffold/src/lib.rs`. Renders the same template files
# (currently shared via `mojo/mohaus_scaffold/templates/`, byte-for-byte
# identical to the Rust crate's `src/templates/`) using a simple `{{name}}`
# substitution loop.
#
# `ScaffoldOptions.templates_dir` is supplied by the caller because Mojo
# doesn't expose `__file__`. Production usage passes the path of
# `src/mohaus_scaffold/templates/` (or wherever the Mojo `.mojopkg` was
# packaged with its template payload).

from std.collections import List
from std.os import listdir, makedirs
from std.os.path import isdir
from std.pathlib import Path

comptime DEFAULT_MOJO_VERSION = "1.0.0b2.dev2026050805"


@fieldwise_init
struct ScaffoldOptions(Movable):
    var name: String
    var destination: String
    var templates_dir: String


def _python_identifier(value: String) -> String:
    var bytes = value.as_bytes()
    var out = List[UInt8]()
    for i in range(len(bytes)):
        var b = bytes[i]
        if b == UInt8(45) or b == UInt8(46):
            out.append(UInt8(95))
        else:
            out.append(b)
    out.append(UInt8(0))
    return String(unsafe_from_utf8_ptr=out.unsafe_ptr())


def _render(template: String, replacements: List[Tuple[String, String]]) -> String:
    var output = template
    for index in range(len(replacements)):
        var entry = replacements[index]
        var needle = entry[0]
        var replacement = entry[1]
        output = output.replace(needle, replacement)
    return output


def _read_template(templates_dir: String, name: String) raises -> String:
    var template_path = Path(templates_dir).joinpath(name)
    return template_path.read_text()


def _write_file(parent_dir: String, file_name: String, body: String) raises:
    makedirs(parent_dir, exist_ok=True)
    var path = Path(parent_dir).joinpath(file_name)
    var handle = open(String(path), "w")
    handle.write(body)
    handle.close()


def scaffold_project(options: ScaffoldOptions) raises:
    var destination_path = Path(options.destination)
    if destination_path.exists():
        if not isdir(String(destination_path)):
            raise Error(
                "destination exists and is not a directory: ",
                String(destination_path),
            )
        var entries = listdir(String(destination_path))
        if len(entries) > 0:
            raise Error("destination is not empty: ", String(destination_path))
    else:
        makedirs(String(destination_path), exist_ok=False)

    var import_name = _python_identifier(options.name)
    var replacements = List[Tuple[String, String]]()
    replacements.append(Tuple[String, String]("{{project_name}}", options.name))
    replacements.append(Tuple[String, String]("{{import_name}}", import_name))
    replacements.append(Tuple[String, String]("{{mojo_version}}", DEFAULT_MOJO_VERSION))

    var dest_str = String(destination_path)
    var python_root = String(destination_path.joinpath("python").joinpath(import_name))
    var src_root = String(destination_path.joinpath("src"))

    _write_file(
        dest_str,
        "pyproject.toml",
        _render(_read_template(options.templates_dir, "pyproject.toml.tmpl"), replacements),
    )
    _write_file(
        dest_str,
        "flake.nix",
        _render(_read_template(options.templates_dir, "flake.nix.tmpl"), replacements),
    )
    _write_file(
        src_root,
        "lib.mojo",
        _render(_read_template(options.templates_dir, "lib.mojo.tmpl"), replacements),
    )
    _write_file(
        python_root,
        "__init__.py",
        _render(_read_template(options.templates_dir, "__init__.py.tmpl"), replacements),
    )
    _write_file(python_root, "py.typed", "")
    _write_file(
        dest_str,
        "README.md",
        _render(_read_template(options.templates_dir, "README.md.tmpl"), replacements),
    )
    _write_file(
        dest_str,
        ".gitignore",
        _render(_read_template(options.templates_dir, "gitignore.tmpl"), replacements),
    )
    _write_file(
        dest_str,
        ".gitattributes",
        _render(_read_template(options.templates_dir, "gitattributes.tmpl"), replacements),
    )
    _write_file(
        dest_str,
        "LICENSE",
        _render(_read_template(options.templates_dir, "LICENSE.tmpl"), replacements),
    )
    _write_file(dest_str, ".mojo-version", DEFAULT_MOJO_VERSION)
