use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::config::ProjectConfig;
use crate::editable::{compile_module, ensure_editable_built};
use crate::error::{MohausError, Result};
use crate::python_info::PythonInfo;
use crate::sdist::write_sdist_archive;
use crate::toolchain::resolve_verified_mojo;
use crate::wheel::{
    copy_dir, copy_prepared_dist_info, write_dist_info, write_file, write_wheel_archive,
};

/// Options for building a wheel.
#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub project_dir: PathBuf,
    pub out_dir: PathBuf,
    pub python: PythonInfo,
    pub release: bool,
    /// PEP 517 `metadata_directory` previously populated by
    /// `prepare_metadata_for_build_wheel`. When present, mohaus reuses the
    /// dist-info contents instead of regenerating them, satisfying the
    /// byte-for-byte requirement of the protocol.
    pub metadata_dir: Option<PathBuf>,
}

/// Options for building an editable wheel.
#[derive(Clone, Debug)]
pub struct EditableOptions {
    pub project_dir: PathBuf,
    pub out_dir: PathBuf,
    pub python: PythonInfo,
    /// PEP 660 `metadata_directory` previously populated by
    /// `prepare_metadata_for_build_editable`.
    pub metadata_dir: Option<PathBuf>,
}

/// Options for building an sdist.
#[derive(Clone, Debug)]
pub struct SdistOptions {
    pub project_dir: PathBuf,
    pub out_dir: PathBuf,
}

/// Options for preparing wheel metadata.
#[derive(Clone, Debug)]
pub struct MetadataOptions {
    pub project_dir: PathBuf,
    pub metadata_dir: PathBuf,
    pub python: PythonInfo,
}

/// Build a host wheel for a mohaus project.
///
/// # Errors
///
/// Returns an error when configuration, Mojo resolution, compilation, staging,
/// metadata generation, or wheel writing fails.
pub fn build_wheel(options: &BuildOptions) -> Result<PathBuf> {
    let config = ProjectConfig::load(&options.project_dir)?;
    let mojo = resolve_verified_mojo(&config.mojo_version)?;
    let staged = TempDir::new().map_err(|source| MohausError::CreateDir {
        path: options.project_dir.join("target/mohaus-staged"),
        source,
    })?;

    stage_python_tree(&config, staged.path())?;
    for module in &config.modules {
        let output =
            extension_output_path_in_root(staged.path(), module, &options.python.ext_suffix);
        compile_module(&config, module, &output, &mojo.executable)?;
    }

    let reused = match &options.metadata_dir {
        Some(metadata_dir) => copy_prepared_dist_info(metadata_dir, staged.path(), &config)?,
        None => false,
    };
    if !reused {
        write_dist_info(
            staged.path(),
            &config,
            &options.python.wheel_tag,
            false,
            false,
        )?;
    }
    let wheel_name = format!(
        "{}-{}-{}.whl",
        config.package.escaped(),
        config.version,
        options.python.wheel_tag
    );
    let wheel_path = options.out_dir.join(wheel_name);
    write_wheel_archive(staged.path(), &wheel_path, &config.dist_info_dir())?;
    Ok(wheel_path)
}

/// Build a PEP 660 editable wheel.
///
/// # Errors
///
/// Returns an error when configuration loading, in-place compilation, editable
/// metadata generation, or wheel writing fails.
pub fn build_editable_wheel(options: &EditableOptions) -> Result<PathBuf> {
    let config = ProjectConfig::load(&options.project_dir)?;
    ensure_editable_built(&options.project_dir, &options.python)?;
    let staged = TempDir::new().map_err(|source| MohausError::CreateDir {
        path: options.project_dir.join("target/mohaus-editable-staged"),
        source,
    })?;
    let pth_name = format!("zz_mohaus_{}_editable.pth", config.package.escaped());
    let pth = format!(
        "{}\nimport importlib, importlib.util; spec = importlib.util.find_spec('mohaus._editable'); spec and importlib.import_module('mohaus._editable').ensure({})\n",
        config.python_source_root().display(),
        python_string_literal(&config.project_dir)
    );
    write_file(&staged.path().join(pth_name), pth.as_bytes())?;
    let reused = match &options.metadata_dir {
        Some(metadata_dir) => copy_prepared_dist_info(metadata_dir, staged.path(), &config)?,
        None => false,
    };
    if !reused {
        write_dist_info(staged.path(), &config, &options.python.pure_tag, true, true)?;
    }
    let wheel_name = format!(
        "{}-{}-{}.whl",
        config.package.escaped(),
        config.version,
        options.python.pure_tag
    );
    let wheel_path = options.out_dir.join(wheel_name);
    write_wheel_archive(staged.path(), &wheel_path, &config.dist_info_dir())?;
    Ok(wheel_path)
}

/// Build a source distribution.
///
/// # Errors
///
/// Returns an error when configuration loading or archive creation fails.
pub fn build_sdist(options: &SdistOptions) -> Result<PathBuf> {
    let config = ProjectConfig::load(&options.project_dir)?;
    write_sdist_archive(&config, &options.out_dir)
}

/// Prepare PEP 517 wheel metadata without building extensions.
///
/// # Errors
///
/// Returns an error when configuration loading or dist-info writing fails.
pub fn prepare_metadata_for_build_wheel(options: &MetadataOptions) -> Result<String> {
    let config = ProjectConfig::load(&options.project_dir)?;
    let dist_info = write_dist_info(
        &options.metadata_dir,
        &config,
        &options.python.wheel_tag,
        false,
        false,
    )?;
    dist_info_name(&dist_info)
}

/// Prepare PEP 660 editable metadata without building extensions.
///
/// # Errors
///
/// Returns an error when configuration loading or dist-info writing fails.
pub fn prepare_metadata_for_build_editable(options: &MetadataOptions) -> Result<String> {
    let config = ProjectConfig::load(&options.project_dir)?;
    let dist_info = write_dist_info(
        &options.metadata_dir,
        &config,
        &options.python.pure_tag,
        true,
        true,
    )?;
    dist_info_name(&dist_info)
}

fn dist_info_name(dist_info: &Path) -> Result<String> {
    let Some(name) = dist_info.file_name().and_then(|value| value.to_str()) else {
        return Err(MohausError::WheelMetadata {
            message: format!(
                "dist-info path has no valid file name: {}",
                dist_info.display()
            ),
        });
    };
    Ok(name.to_string())
}

fn stage_python_tree(config: &ProjectConfig, staged_root: &Path) -> Result<()> {
    let source = config.python_source_root();
    if !source.is_dir() {
        return Err(MohausError::InvalidProject {
            message: format!(
                "python source directory does not exist: {}",
                source.display()
            ),
        });
    }
    copy_dir(&source, staged_root)
}

fn extension_output_path_in_root(
    root: &Path,
    module: &crate::config::MojoModule,
    ext_suffix: &str,
) -> PathBuf {
    let mut relative = module.name.relative_path_without_suffix();
    relative.set_file_name(format!("{}{}", module.name.leaf(), ext_suffix));
    root.join(relative)
}

fn python_string_literal(path: &Path) -> String {
    let text = path.display().to_string();
    format!("{text:?}")
}
