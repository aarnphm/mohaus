from std.os import abort
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

from some_pkg.helpers import shout


@export
def PyInit__native() -> PythonObject:
    try:
        var module = PythonModuleBuilder("_native")
        module.def_function[passthrough]("passthrough")
        return module.finalize()
    except e:
        abort(String("failed to create Python module: ", e))


def passthrough(value: PythonObject) raises -> PythonObject:
    return shout(value)
