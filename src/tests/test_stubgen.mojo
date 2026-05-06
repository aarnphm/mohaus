# Parity tests for mohaus_stubgen. The fixture shape mirrors the Rust tests in
# `crates/mohaus-core/src/stub.rs`; the runtime path stays Rust-owned.

from mohaus_stubgen import parse_binding_source, render_stub_text
from std.testing import assert_equal


def test_scaffold_style_binding_stub() raises:
    var source = (
        "from std.python import PythonObject\n"
        + "from std.python.bindings import PythonModuleBuilder\n"
        + "\n"
        + "@export\n"
        + "def PyInit__native() -> PythonObject:\n"
        + '    var module = PythonModuleBuilder("_native")\n'
        + "    _ = (\n"
        + '        module.def_function[passthrough]("passthrough")\n'
        + '        .add_type[Greeter]("Greeter")\n'
        + "        .def_py_init[Greeter.py_init]()\n"
        + '        .def_method[Greeter.greet]("greet")\n'
        + "    )\n"
        + "    return module.finalize()\n"
        + "\n"
        + "def passthrough(value: PythonObject) raises -> PythonObject:\n"
        + "    return value\n"
        + "\n"
        + "struct Greeter(Movable, Writable):\n"
        + "    @staticmethod\n"
        + "    def py_init(out self: Greeter, args: PythonObject, kwargs: PythonObject) raises:\n"
        + "        self = Self(String(py=args[0]))\n"
        + "\n"
        + "    @staticmethod\n"
        + "    def greet(py_self: PythonObject, value: PythonObject) raises -> PythonObject:\n"
        + "        return value\n"
    )
    assert_equal(
        render_stub_text(source),
        "def passthrough(value: object) -> object: ...\n\n"
        + "class Greeter:\n"
        + "  def __init__(self, *args: object, **kwargs: object) -> None: ...\n"
        + "  def greet(self, value: object) -> object: ...\n",
    )


def test_keyword_static_and_default_init_stub() raises:
    var source = (
        "from std.collections import OwnedKwargsDict\n"
        + "from std.python import PythonObject\n"
        + "from std.python.bindings import PythonModuleBuilder\n"
        + "\n"
        + "def duration(hours: PythonObject, kwargs: OwnedKwargsDict[PythonObject]) raises -> PythonObject:\n"
        + "    return hours\n"
        + "\n"
        + "struct Timer(Defaultable, Movable, Writable):\n"
        + "    @staticmethod\n"
        + "    def is_valid(value: PythonObject) raises -> PythonObject:\n"
        + "        return value\n"
        + "\n"
        + "@export\n"
        + "def PyInit__native() -> PythonObject:\n"
        + '    var module = PythonModuleBuilder("_native")\n'
        + '    module.def_function[duration]("duration")\n'
        + "    _ = (\n"
        + '        module.add_type[Timer]("Timer")\n'
        + "        .def_init_defaultable[Timer]()\n"
        + '        .def_staticmethod[Timer.is_valid]("is_valid")\n'
        + "    )\n"
        + "    return module.finalize()\n"
    )
    assert_equal(
        render_stub_text(source),
        "def duration(hours: object, **kwargs: object) -> object: ...\n\n"
        + "class Timer:\n"
        + "  def __init__(self) -> None: ...\n"
        + "  @staticmethod\n"
        + "  def is_valid(value: object) -> object: ...\n",
    )


def test_low_level_bindings_stay_broad() raises:
    var source = (
        "from std.python import PythonObject\n"
        + "from std.python.bindings import PythonModuleBuilder\n"
        + "\n"
        + "def count_args(py_self: PythonObject, args_tuple: PythonObject) raises -> PythonObject:\n"
        + "    return args_tuple\n"
        + "\n"
        + "@export\n"
        + "def PyInit__native() -> PythonObject:\n"
        + '    var module = PythonModuleBuilder("_native")\n'
        + '    module.def_py_function[count_args]("count_args")\n'
        + '    module.def_py_c_function(raw_count, "raw_count")\n'
        + "    _ = (\n"
        + '        module.add_type[Counter]("Counter")\n'
        + '        .def_py_method[Counter.lookup]("lookup")\n'
        + '        .def_py_c_method(raw_lookup, "raw_lookup")\n'
        + "    )\n"
        + "    return module.finalize()\n"
        + "\n"
        + "struct Counter(Movable, Writable):\n"
        + "    @staticmethod\n"
        + "    def lookup(py_self: PythonObject, args_tuple: PythonObject) raises -> PythonObject:\n"
        + "        return args_tuple\n"
    )
    assert_equal(
        render_stub_text(source),
        "def count_args(*args: object, **kwargs: object) -> object: ...\n"
        + "def raw_count(*args: object, **kwargs: object) -> object: ...\n\n"
        + "class Counter:\n"
        + "  def lookup(self, *args: object, **kwargs: object) -> object: ...\n"
        + "  def raw_lookup(self, *args: object, **kwargs: object) -> object: ...\n",
    )


def test_chained_builder_calls_can_span_lines() raises:
    var source = (
        "from std.python import PythonObject\n"
        + "from std.python.bindings import PythonModuleBuilder\n"
        + "\n"
        + "def passthrough(value: PythonObject) raises -> PythonObject:\n"
        + "    return value\n"
        + "\n"
        + "@export\n"
        + "def PyInit__native() -> PythonObject:\n"
        + '    var module = PythonModuleBuilder("_native")\n'
        + "    _ = (\n"
        + "        module\n"
        + "            .def_function[\n"
        + "                passthrough,\n"
        + "            ](\n"
        + '                "passthrough",\n'
        + "            )\n"
        + "    )\n"
        + "    return module.finalize()\n"
    )
    var parsed = parse_binding_source(source)
    assert_equal(len(parsed.calls), 1)
    assert_equal(
        render_stub_text(source),
        "def passthrough(value: object) -> object: ...\n",
    )


def main() raises:
    test_scaffold_style_binding_stub()
    test_keyword_static_and_default_init_stub()
    test_low_level_bindings_stay_broad()
    test_chained_builder_calls_can_span_lines()
