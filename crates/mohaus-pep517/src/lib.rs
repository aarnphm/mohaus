//! PyO3 bridge for the mohaus PEP 517 backend.

use std::path::{Path, PathBuf};

use mohaus_core::editable::{source_hash, tree_hash};
use mohaus_core::{
    BuildOptions, EditableOptions, MetadataOptions, ProjectConfig, PythonInfo, SdistOptions,
    Verbosity, build_editable_wheel, build_sdist as core_build_sdist,
    build_wheel as core_build_wheel, discover_mojo_executable_from_python_scripts,
    discover_mojo_paths_from_python_roots, ensure_editable_built_with_verbosity,
    prepare_metadata_for_build_editable as core_prepare_editable_metadata,
    prepare_metadata_for_build_wheel as core_prepare_metadata,
};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyModule;
use pyo3::wrap_pyfunction;

#[pyfunction]
#[pyo3(signature = (wheel_directory, config_settings=None, metadata_directory=None))]
fn build_wheel(
    py: Python<'_>,
    wheel_directory: String,
    config_settings: Option<Py<PyAny>>,
    metadata_directory: Option<String>,
) -> PyResult<String> {
    let _ = config_settings;
    let project_dir = current_dir_py()?;
    let python = python_info(py)?;
    let path = core_build_wheel(&BuildOptions {
        project_dir,
        out_dir: PathBuf::from(wheel_directory),
        python,
        release: true,
        verbosity: Verbosity::from_env(),
        metadata_dir: metadata_directory.map(PathBuf::from),
    })
    .map_err(to_py_error)?;
    file_name_string(&path)
}

#[pyfunction]
#[pyo3(signature = (sdist_directory, config_settings=None))]
fn build_sdist(sdist_directory: String, config_settings: Option<Py<PyAny>>) -> PyResult<String> {
    let _ = config_settings;
    let project_dir = current_dir_py()?;
    let path = core_build_sdist(&SdistOptions {
        project_dir,
        out_dir: PathBuf::from(sdist_directory),
    })
    .map_err(to_py_error)?;
    file_name_string(&path)
}

#[pyfunction]
#[pyo3(signature = (wheel_directory, config_settings=None, metadata_directory=None))]
fn build_editable(
    py: Python<'_>,
    wheel_directory: String,
    config_settings: Option<Py<PyAny>>,
    metadata_directory: Option<String>,
) -> PyResult<String> {
    let _ = config_settings;
    let project_dir = current_dir_py()?;
    let python = python_info(py)?;
    let path = build_editable_wheel(&EditableOptions {
        project_dir,
        out_dir: PathBuf::from(wheel_directory),
        python,
        verbosity: Verbosity::from_env(),
        metadata_dir: metadata_directory.map(PathBuf::from),
    })
    .map_err(to_py_error)?;
    file_name_string(&path)
}

#[pyfunction]
#[pyo3(signature = (config_settings=None))]
fn get_requires_for_build_wheel(config_settings: Option<Py<PyAny>>) -> PyResult<Vec<String>> {
    let _ = config_settings;
    let project_dir = current_dir_py()?;
    mojo_build_requirements(&project_dir).map_err(to_py_error)
}

#[pyfunction]
#[pyo3(signature = (config_settings=None))]
fn get_requires_for_build_editable(config_settings: Option<Py<PyAny>>) -> PyResult<Vec<String>> {
    let _ = config_settings;
    let project_dir = current_dir_py()?;
    mojo_build_requirements(&project_dir).map_err(to_py_error)
}

#[pyfunction]
#[pyo3(signature = (metadata_directory, config_settings=None))]
fn prepare_metadata_for_build_wheel(
    py: Python<'_>,
    metadata_directory: String,
    config_settings: Option<Py<PyAny>>,
) -> PyResult<String> {
    let _ = config_settings;
    let project_dir = current_dir_py()?;
    let python = python_info(py)?;
    core_prepare_metadata(&MetadataOptions {
        project_dir,
        metadata_dir: PathBuf::from(metadata_directory),
        python,
        verbosity: Verbosity::from_env(),
    })
    .map_err(to_py_error)
}

#[pyfunction]
#[pyo3(signature = (metadata_directory, config_settings=None))]
fn prepare_metadata_for_build_editable(
    py: Python<'_>,
    metadata_directory: String,
    config_settings: Option<Py<PyAny>>,
) -> PyResult<String> {
    let _ = config_settings;
    let project_dir = current_dir_py()?;
    let python = python_info(py)?;
    core_prepare_editable_metadata(&MetadataOptions {
        project_dir,
        metadata_dir: PathBuf::from(metadata_directory),
        python,
        verbosity: Verbosity::from_env(),
    })
    .map_err(to_py_error)
}

#[pyfunction]
fn rebuild_editable(py: Python<'_>, project_root: String) -> PyResult<()> {
    let python = python_info(py)?;
    ensure_editable_built_with_verbosity(project_root, &python, Verbosity::from_env())
        .map_err(to_py_error)
}

#[pyfunction]
fn source_hash_for_project(project_root: String) -> PyResult<String> {
    let config = ProjectConfig::load(PathBuf::from(&project_root)).map_err(to_py_error)?;
    let module = config
        .modules
        .first()
        .ok_or_else(|| PyRuntimeError::new_err("project has no Mojo modules configured"))?;
    source_hash(&config, module).map_err(to_py_error)
}

#[pyfunction]
fn tree_hash_for_dir(root: String) -> PyResult<String> {
    tree_hash(&PathBuf::from(root)).map_err(to_py_error)
}

#[pyfunction]
fn cli(argv: Vec<String>) -> i32 {
    let args = std::iter::once("mohaus".to_string()).chain(argv);
    match mohaus_cli::run_from(args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    }
}

#[pymodule]
fn mohaus_pep517(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(build_wheel, m)?)?;
    m.add_function(wrap_pyfunction!(build_sdist, m)?)?;
    m.add_function(wrap_pyfunction!(build_editable, m)?)?;
    m.add_function(wrap_pyfunction!(get_requires_for_build_wheel, m)?)?;
    m.add_function(wrap_pyfunction!(get_requires_for_build_editable, m)?)?;
    m.add_function(wrap_pyfunction!(prepare_metadata_for_build_wheel, m)?)?;
    m.add_function(wrap_pyfunction!(prepare_metadata_for_build_editable, m)?)?;
    m.add_function(wrap_pyfunction!(rebuild_editable, m)?)?;
    m.add_function(wrap_pyfunction!(source_hash_for_project, m)?)?;
    m.add_function(wrap_pyfunction!(tree_hash_for_dir, m)?)?;
    m.add_function(wrap_pyfunction!(cli, m)?)?;
    Ok(())
}

fn python_info(py: Python<'_>) -> PyResult<PythonInfo> {
    let sysconfig = py.import("sysconfig")?;
    let sys = py.import("sys")?;
    let ext_suffix = sysconfig
        .call_method1("get_config_var", ("EXT_SUFFIX",))?
        .extract::<Option<String>>()?
        .unwrap_or_else(|| ".so".to_string());
    let cache_tag = sys
        .getattr("implementation")?
        .getattr("cache_tag")?
        .extract::<Option<String>>()?
        .unwrap_or_else(|| "py3".to_string());
    let platform = sysconfig
        .call_method0("get_platform")?
        .extract::<String>()?;
    let version_info = sys.getattr("version_info")?;
    let major = version_info.getattr("major")?.extract::<u8>()?;
    let minor = version_info.getattr("minor")?.extract::<u8>()?;
    let (mojo_executables, mojo_search_paths) = python_mojo_paths(&sysconfig, &sys)?;
    PythonInfo::from_parts_with_mojo_paths(
        ext_suffix,
        cache_tag,
        platform,
        major,
        minor,
        mojo_executables,
        mojo_search_paths,
    )
    .map_err(to_py_error)
}

fn python_mojo_paths(
    sysconfig: &Bound<'_, PyModule>,
    sys: &Bound<'_, PyModule>,
) -> PyResult<(Vec<PathBuf>, Vec<PathBuf>)> {
    let mut roots = Vec::new();
    let scripts = sysconfig
        .call_method1("get_path", ("scripts",))?
        .extract::<Option<String>>()?
        .map(PathBuf::from);
    let paths = sysconfig.call_method0("get_paths")?;
    for key in ["purelib", "platlib"] {
        if let Ok(Some(path)) = paths
            .get_item(key)
            .and_then(|value| value.extract::<Option<String>>())
        {
            roots.push(PathBuf::from(path));
        }
    }
    for path in sys.getattr("path")?.extract::<Vec<String>>()? {
        if !path.is_empty() {
            roots.push(PathBuf::from(path));
        }
    }
    let (mut executables, search_paths) = discover_mojo_paths_from_python_roots(roots);
    let mut script_executables = discover_mojo_executable_from_python_scripts(scripts);
    script_executables.append(&mut executables);
    Ok((script_executables, search_paths))
}

fn current_dir_py() -> PyResult<PathBuf> {
    std::env::current_dir().map_err(|source| {
        PyRuntimeError::new_err(format!("could not read current directory: {source}"))
    })
}

fn mojo_build_requirements(project_dir: &Path) -> mohaus_core::Result<Vec<String>> {
    let config = ProjectConfig::load(project_dir)?;
    if config.pure {
        return Ok(Vec::new());
    }
    let Some(version) = config.mojo_version.as_ref() else {
        return Ok(Vec::new());
    };
    let version = version.as_str();
    Ok(vec![
        format!("mojo=={version}"),
        format!("mojo-compiler=={version}"),
        format!("mojo-compiler-mojo-libs=={version}"),
        format!("mojo-lldb-libs=={version}"),
    ])
}

fn file_name_string(path: &Path) -> PyResult<String> {
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return Err(PyRuntimeError::new_err(format!(
            "path has no valid file name: {}",
            path.display()
        )));
    };
    Ok(file_name.to_string())
}

fn to_py_error(error: mohaus_core::MohausError) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use crate::mojo_build_requirements;

    fn write_pyproject(root: &TempDir, tool_mohaus: &str) {
        fs::write(
            root.path().join("pyproject.toml"),
            format!(
                "[project]\n\
                 name = \"demo\"\n\
                 version = \"0.1.0\"\n\
                 requires-python = \">=3.11\"\n\
                 \n\
                 [tool.mohaus]\n\
                 {tool_mohaus}\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn pinned_mojo_version_becomes_build_requirements() {
        let root = TempDir::new().unwrap();
        write_pyproject(&root, "module-name = \"demo._native\"\n");
        fs::write(root.path().join(".mojo-version"), "1.0.0b1").unwrap();

        assert_eq!(
            mojo_build_requirements(root.path()).unwrap(),
            vec![
                "mojo==1.0.0b1",
                "mojo-compiler==1.0.0b1",
                "mojo-compiler-mojo-libs==1.0.0b1",
                "mojo-lldb-libs==1.0.0b1",
            ]
        );
    }

    #[test]
    fn unpinned_or_pure_projects_add_no_mojo_build_requirements() {
        let unpinned = TempDir::new().unwrap();
        write_pyproject(&unpinned, "module-name = \"demo._native\"\n");
        assert!(mojo_build_requirements(unpinned.path()).unwrap().is_empty());

        let pure = TempDir::new().unwrap();
        write_pyproject(&pure, "pure = true\n");
        fs::write(pure.path().join(".mojo-version"), "1.0.0b1").unwrap();
        assert!(mojo_build_requirements(pure.path()).unwrap().is_empty());
    }
}
