//! PyO3 bridge for the mohaus PEP 517 backend.

use std::path::PathBuf;

use mohaus_core::{
    BuildOptions, EditableOptions, MetadataOptions, PythonInfo, SdistOptions, build_editable_wheel,
    build_sdist as core_build_sdist, build_wheel as core_build_wheel, ensure_editable_built,
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
        metadata_dir: metadata_directory.map(PathBuf::from),
    })
    .map_err(to_py_error)?;
    file_name_string(&path)
}

#[pyfunction]
#[pyo3(signature = (config_settings=None))]
fn get_requires_for_build_wheel(config_settings: Option<Py<PyAny>>) -> Vec<String> {
    let _ = config_settings;
    Vec::new()
}

#[pyfunction]
#[pyo3(signature = (config_settings=None))]
fn get_requires_for_build_editable(config_settings: Option<Py<PyAny>>) -> Vec<String> {
    let _ = config_settings;
    Vec::new()
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
    })
    .map_err(to_py_error)
}

#[pyfunction]
fn rebuild_editable(py: Python<'_>, project_root: String) -> PyResult<()> {
    let python = python_info(py)?;
    ensure_editable_built(project_root, &python).map_err(to_py_error)
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
    PythonInfo::from_parts(ext_suffix, cache_tag, platform, major, minor).map_err(to_py_error)
}

fn current_dir_py() -> PyResult<PathBuf> {
    std::env::current_dir().map_err(|source| {
        PyRuntimeError::new_err(format!("could not read current directory: {source}"))
    })
}

fn file_name_string(path: &std::path::Path) -> PyResult<String> {
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
