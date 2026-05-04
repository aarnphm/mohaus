from std.os import abort
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder


@export
def PyInit__extra() -> PythonObject:
    try:
        var module = PythonModuleBuilder("_extra")
        module.def_function[bonus]("bonus")
        return module.finalize()
    except e:
        abort(String("failed to create Python module: ", e))


def bonus(value: PythonObject) raises -> PythonObject:
    return value + " bonus"
