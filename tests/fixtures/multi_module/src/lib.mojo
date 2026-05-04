from std.os import abort
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder


@export
def PyInit__native() -> PythonObject:
    try:
        var module = PythonModuleBuilder("_native")
        module.def_function[hello]("hello")
        return module.finalize()
    except e:
        abort(String("failed to create Python module: ", e))


def hello(value: PythonObject) raises -> PythonObject:
    return value + " hello"
