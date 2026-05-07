use std::fs;
use std::path::{Path, PathBuf};

use mohaus_mojo_bindings::{ModuleName, SourceGraph, SourceInput, analyze_python_bindings};
use mohaus_stubgen::render_pyi;

use crate::config::{MojoModule, ProjectConfig};
use crate::error::{MohausError, Result};
use crate::wheel::write_file;

/// A generated Python stub for one configured Mojo extension module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleStub {
    pub path: PathBuf,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StubSource {
    module_name: Option<String>,
    text: String,
    is_entry: bool,
}

/// Return the `.pyi` output path for a module compiled to `extension_output`.
///
/// # Errors
///
/// Returns an error when the extension output path has no parent directory.
pub fn module_stub_path_for_extension(
    module: &MojoModule,
    extension_output: &Path,
) -> Result<PathBuf> {
    let Some(output_dir) = extension_output.parent() else {
        return Err(MohausError::InvalidProject {
            message: format!(
                "extension output has no parent directory: {}",
                extension_output.display()
            ),
        });
    };
    Ok(module_stub_path_in_dir(module, output_dir))
}

/// Build the generated stub path and contents for one configured module.
///
/// # Errors
///
/// Returns an error when the module entry source cannot be read or parsed, or
/// the extension output path has no parent directory.
pub fn module_stub_plan_for_extension(
    config: &ProjectConfig,
    module: &MojoModule,
    extension_output: &Path,
) -> Result<ModuleStub> {
    let path = module_stub_path_for_extension(module, extension_output)?;
    let text = render_module_stub(config, module)?;
    Ok(ModuleStub { path, text })
}

/// Write the generated `.pyi` next to a compiled extension.
///
/// # Errors
///
/// Returns an error when the module entry source cannot be read, parsed, or the
/// stub cannot be written.
pub fn write_module_stub_for_extension(
    config: &ProjectConfig,
    module: &MojoModule,
    extension_output: &Path,
) -> Result<PathBuf> {
    let stub = module_stub_plan_for_extension(config, module, extension_output)?;
    write_file(&stub.path, stub.text.as_bytes())?;
    Ok(stub.path)
}

/// Render a `.pyi` file from Python bindings declared in a Mojo entry file.
///
/// # Errors
///
/// Returns an error when the configured module entry source cannot be read or
/// any exported binding references an unsupported or unresolved declaration.
pub fn render_module_stub(config: &ProjectConfig, module: &MojoModule) -> Result<String> {
    let entry = config.project_dir.join(&module.entry);
    let sources = read_stub_sources(config, &entry)?;
    render_stub_from_sources(&sources)
        .map_err(|message| invalid_stub_source(&entry, module.name.as_str(), message))
}

fn render_stub_from_sources(sources: &[StubSource]) -> std::result::Result<String, String> {
    let source_inputs = sources
        .iter()
        .map(|source| SourceInput {
            module_name: source.module_name.clone().map(ModuleName),
            text: source.text.clone(),
            is_entry: source.is_entry,
        })
        .collect::<Vec<_>>();
    let graph = SourceGraph::parse(&source_inputs)?;
    let bindings = analyze_python_bindings(&graph)?;
    Ok(render_pyi(&bindings))
}

fn module_stub_path_in_dir(module: &MojoModule, output_dir: &Path) -> PathBuf {
    output_dir.join(format!("{}.pyi", module.name.leaf()))
}

fn invalid_stub_source(path: &Path, module_name: &str, message: String) -> MohausError {
    MohausError::InvalidProject {
        message: format!(
            "could not generate Python stub for {module_name} from {}: {message}",
            path.display()
        ),
    }
}

fn read_stub_sources(config: &ProjectConfig, entry: &Path) -> Result<Vec<StubSource>> {
    let mut source_paths = Vec::new();
    let mojo_source_root = config.mojo_source_root();
    collect_mojo_source_files(&mojo_source_root, &mut source_paths)?;
    for include in &config.mojo_include_paths {
        let include_root = if include.is_absolute() {
            include.clone()
        } else {
            config.project_dir.join(include)
        };
        collect_mojo_source_files(&include_root, &mut source_paths)?;
    }
    source_paths.sort();
    source_paths.dedup();

    if !source_paths.iter().any(|path| path == entry) {
        source_paths.push(entry.to_path_buf());
    }

    let mut sources = Vec::new();
    for path in source_paths {
        let text = fs::read_to_string(&path).map_err(|source| MohausError::ReadFile {
            path: path.clone(),
            source,
        })?;
        sources.push(StubSource {
            module_name: module_name_for_source(&mojo_source_root, &path),
            text,
            is_entry: path == entry,
        });
    }
    Ok(sources)
}

fn collect_mojo_source_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    match fs::metadata(root) {
        Ok(metadata) if metadata.is_file() => {
            if is_mojo_source(root) {
                out.push(root.to_path_buf());
            }
            Ok(())
        }
        Ok(metadata) if metadata.is_dir() => {
            collect_mojo_source_dir(root, out)?;
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(MohausError::ReadFile {
            path: root.to_path_buf(),
            source,
        }),
    }
}

fn collect_mojo_source_dir(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(root).map_err(|source| MohausError::ReadFile {
        path: root.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| MohausError::ReadFile {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| MohausError::ReadFile {
            path: path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            collect_mojo_source_dir(&path, out)?;
        } else if file_type.is_file() && is_mojo_source(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_mojo_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "mojo" | "🔥"))
}

fn module_name_for_source(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = relative
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let last = parts.last_mut()?;
    if let Some(stem) = Path::new(last.as_str()).file_stem() {
        *last = stem.to_string_lossy().to_string();
    }
    let module = parts
        .into_iter()
        .filter(|part| part != "__init__")
        .collect::<Vec<_>>()
        .join(".");
    (!module.is_empty()).then_some(module)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use crate::config::ProjectConfig;
    use crate::stub::{
        StubSource, module_stub_plan_for_extension, render_module_stub, render_stub_from_sources,
    };

    fn render_stub_from_source(source: &str) -> std::result::Result<String, String> {
        render_stub_from_sources(&[StubSource {
            module_name: None,
            text: source.to_string(),
            is_entry: true,
        }])
    }

    #[test]
    fn renders_function_and_class_bindings() {
        let text = render_stub_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    _ = (
        module.def_function[passthrough]("passthrough")
        .add_type[Greeter]("Greeter")
        .def_py_init[Greeter.py_init]()
        .def_method[Greeter.greet]("greet")
    )
    return module.finalize()

def passthrough(value: PythonObject) raises -> PythonObject:
    return value

@fieldwise_init
struct Greeter(Movable, Writable):
    var suffix: String

    @staticmethod
    def py_init(out self: Greeter, args: PythonObject, kwargs: PythonObject) raises:
        self = Self(String(py=args[0]))

    @staticmethod
    def greet(py_self: PythonObject, value: PythonObject) raises -> PythonObject:
        return value
"#,
        )
        .unwrap();

        assert_eq!(
            text,
            concat!(
                "def passthrough(value: object) -> object: ...\n\n",
                "class Greeter:\n",
                "  def __init__(self, *args: object, **kwargs: object) -> None: ...\n",
                "  def greet(self, value: object) -> object: ...\n",
            )
        );
    }

    #[test]
    fn renders_staticmethod_default_init_and_keyword_dict() {
        let text = render_stub_from_source(
            r#"
from std.collections import OwnedKwargsDict
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

def duration(hours: PythonObject, kwargs: OwnedKwargsDict[PythonObject]) raises -> PythonObject:
    return hours

struct Timer(Defaultable, Movable, Writable):
    @staticmethod
    def is_valid(value: PythonObject) raises -> PythonObject:
        return value

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_function[duration]("duration")
    _ = (
        module.add_type[Timer]("Timer")
        .def_init_defaultable[Timer]()
        .def_staticmethod[Timer.is_valid]("is_valid")
    )
    return module.finalize()
"#,
        )
        .unwrap();

        assert_eq!(
            text,
            concat!(
                "def duration(hours: object, **kwargs: object) -> object: ...\n\n",
                "class Timer:\n",
                "  def __init__(self) -> None: ...\n",
                "  @staticmethod\n",
                "  def is_valid(value: object) -> object: ...\n",
            )
        );
    }

    #[test]
    fn broadens_low_level_py_and_c_bindings_to_object_varargs() {
        let text = render_stub_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

def count_args(py_self: PythonObject, args_tuple: PythonObject) raises -> PythonObject:
    return args_tuple

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_py_function[count_args]("count_args")
    module.def_py_c_function(raw_count, "raw_count")
    _ = (
        module.add_type[Counter]("Counter")
        .def_py_method[Counter.lookup]("lookup")
        .def_py_c_method(raw_lookup, "raw_lookup")
    )
    return module.finalize()

struct Counter(Movable, Writable):
    @staticmethod
    def lookup(py_self: PythonObject, args_tuple: PythonObject) raises -> PythonObject:
        return args_tuple
"#,
        )
        .unwrap();

        assert_eq!(
            text,
            concat!(
                "def count_args(*args: object, **kwargs: object) -> object: ...\n",
                "def raw_count(*args: object, **kwargs: object) -> object: ...\n\n",
                "class Counter:\n",
                "  def lookup(self, *args: object, **kwargs: object) -> object: ...\n",
                "  def raw_lookup(self, *args: object, **kwargs: object) -> object: ...\n",
            )
        );
    }

    #[test]
    fn parses_multiline_chained_builder_calls() {
        let text = render_stub_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

def passthrough(value: PythonObject) raises -> PythonObject:
    return value

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    _ = (
        module
            .def_function[
                passthrough,
            ](
                "passthrough",
            )
    )
    return module.finalize()
"#,
        )
        .unwrap();

        assert_eq!(text, "def passthrough(value: object) -> object: ...\n");
    }

    #[test]
    fn parses_generic_def_headers_without_treating_brackets_as_names() {
        let text = render_stub_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

def passthrough[T: CollectionElement](value: PythonObject) raises -> PythonObject:
    return value

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_function[passthrough]("passthrough")
    return module.finalize()
"#,
        )
        .unwrap();

        assert_eq!(text, "def passthrough(value: object) -> object: ...\n");
    }

    #[test]
    fn infers_pythonobject_wrapper_return_types() {
        let text = render_stub_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder
from std.collections import List

struct Layout:
    var shape: Shape[Self.rank]

struct Array(Movable, Writable):
    var dtype_code: Int
    var shape: List[Int]

    @staticmethod
    def dtype_code_py(py_self: PythonObject) raises -> PythonObject:
        var self_ptr = py_self.downcast_value_ptr[Self]()
        return PythonObject(self_ptr[].dtype_code)

    @staticmethod
    def shape_at_py(py_self: PythonObject, index_obj: PythonObject) raises -> PythonObject:
        var self_ptr = py_self.downcast_value_ptr[Self]()
        var index = Int(py=index_obj)
        return PythonObject(self_ptr[].shape[index])

    @staticmethod
    def get_scalar_py(py_self: PythonObject, index_obj: PythonObject) raises -> PythonObject:
        if True:
            return PythonObject(get_bool())
        if False:
            return PythonObject(get_i64())
        return PythonObject(get_f64())

    @staticmethod
    def used_fast_py(py_self: PythonObject) raises -> PythonObject:
        var self_ptr = py_self.downcast_value_ptr[Self]()
        return PythonObject(self_ptr[].dtype_code == 1)

    @staticmethod
    def is_c_contiguous_py(py_self: PythonObject) raises -> PythonObject:
        return PythonObject(is_c_contiguous())

def is_c_contiguous() -> Bool:
    return True

def make_array() raises -> Array:
    pass

def get_bool() -> Bool:
    return True

def get_i64() -> Int64:
    return 1

def get_f64() -> Float64:
    return 1.0

def empty_ops() raises -> PythonObject:
    var result = make_array()
    return PythonObject(alloc=result^)

def none_ops() raises -> PythonObject:
    return PythonObject(None)

def binary_op_method_ops(py_self: PythonObject, other_obj: PythonObject, op: Int) raises -> PythonObject:
    var result = make_array()
    return PythonObject(alloc=result^)

def array_add_method_ops(py_self: PythonObject, other_obj: PythonObject) raises -> PythonObject:
    return binary_op_method_ops(py_self, other_obj, 0)

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    _ = (
        module.add_type[Array]("Array")
        .def_method[array_add_method_ops]("add")
        .def_method[Array.dtype_code_py]("dtype_code")
        .def_method[Array.shape_at_py]("shape_at")
        .def_method[Array.get_scalar_py]("get_scalar")
        .def_method[Array.used_fast_py]("used_fast")
        .def_method[Array.is_c_contiguous_py]("is_c_contiguous")
    )
    module.def_function[empty_ops]("empty")
    module.def_function[none_ops]("none")
    return module.finalize()
"#,
        )
        .unwrap();

        assert_eq!(
            text,
            concat!(
                "def empty() -> Array: ...\n",
                "def none() -> None: ...\n\n",
                "class Array:\n",
                "  def add(self, other_obj: object) -> Array: ...\n",
                "  def dtype_code(self) -> int: ...\n",
                "  def get_scalar(self, index_obj: object) -> bool | int | float: ...\n",
                "  def is_c_contiguous(self) -> bool: ...\n",
                "  def shape_at(self, index_obj: object) -> int: ...\n",
                "  def used_fast(self) -> bool: ...\n",
            )
        );
    }

    #[test]
    fn unresolved_binding_target_is_an_error() {
        let error = render_stub_from_source(
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_function[missing]("missing")
    return module.finalize()
"#,
        )
        .unwrap_err();

        assert!(error.contains("missing"));
        assert!(error.contains("does not resolve"));
    }

    #[test]
    fn targets_extension_leaf_without_abi_suffix() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::create_dir_all(temp.path().join("python/demo")).unwrap();
        fs::write(temp.path().join(".mojo-version"), "0.26.2.0").unwrap();
        fs::write(
            temp.path().join("pyproject.toml"),
            r#"
[project]
name = "demo"
version = "0.1.0"

[tool.mohaus]
module-name = "demo._native"
"#,
        )
        .unwrap();
        fs::write(temp.path().join("src/lib.mojo"), "").unwrap();

        let config = ProjectConfig::load(temp.path()).unwrap();
        let extension = temp
            .path()
            .join("python/demo/_native.cpython-311-darwin.so");
        let stub = module_stub_plan_for_extension(&config, &config.modules[0], &extension).unwrap();

        assert_eq!(stub.path, temp.path().join("python/demo/_native.pyi"));
        assert_eq!(stub.text, "...\n");
    }

    #[test]
    fn render_module_stub_adds_project_context_to_errors() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::create_dir_all(temp.path().join("python/demo")).unwrap();
        fs::write(temp.path().join(".mojo-version"), "0.26.2.0").unwrap();
        fs::write(
            temp.path().join("pyproject.toml"),
            r#"
[project]
name = "demo"
version = "0.1.0"

[tool.mohaus]
module-name = "demo._native"
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("src/lib.mojo"),
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_function[missing]("missing")
    return module.finalize()
"#,
        )
        .unwrap();

        let config = ProjectConfig::load(temp.path()).unwrap();
        let error = render_module_stub(&config, &config.modules[0]).unwrap_err();

        assert!(error.to_string().contains("demo._native"));
        assert!(error.to_string().contains("missing"));
    }

    #[test]
    fn resolves_binding_targets_from_source_root_modules() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::create_dir_all(temp.path().join("python/demo")).unwrap();
        fs::write(temp.path().join(".mojo-version"), "0.26.2.0").unwrap();
        fs::write(
            temp.path().join("pyproject.toml"),
            r#"
[project]
name = "demo"
version = "0.1.0"

[tool.mohaus]
module-name = "demo._native"
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("src/lib.mojo"),
            r#"
from std.python import PythonObject
from std.python.bindings import PythonModuleBuilder

from array import Array
from create import (
    imported_fun as aliased_fun,
)

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    _ = module.add_type[Array]("Array").def_method[Array.dtype_code_py]("dtype_code")
    module.def_function[aliased_fun]("aliased_fun")
    return module.finalize()
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("src/array.mojo"),
            r#"
from std.python import PythonObject

struct Array(Movable, Writable):
    @staticmethod
    def dtype_code_py(py_self: PythonObject) raises -> PythonObject:
        return PythonObject(1)
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("src/create.mojo"),
            r#"
from std.python import PythonObject

def imported_fun(value: PythonObject) raises -> PythonObject:
    return value
"#,
        )
        .unwrap();

        let config = ProjectConfig::load(temp.path()).unwrap();
        let text = render_module_stub(&config, &config.modules[0]).unwrap();

        assert_eq!(
            text,
            concat!(
                "def aliased_fun(value: object) -> object: ...\n\n",
                "class Array:\n",
                "  def dtype_code(self) -> int: ...\n",
            )
        );
    }
}
