//! Project scaffolding for mohaus.

use std::fs;
use std::path::{Path, PathBuf};

use mohaus_core::DEFAULT_MOJO_VERSION;
use mohaus_core::config::PackageName;
use mohaus_core::error::{MohausError, Result};
use mohaus_core::wheel::write_file;

const PYPROJECT_TEMPLATE: &str = include_str!("templates/pyproject.toml.tmpl");
const LIB_MOJO_TEMPLATE: &str = include_str!("templates/lib.mojo.tmpl");
const PY_INIT_TEMPLATE: &str = include_str!("templates/__init__.py.tmpl");
const README_TEMPLATE: &str = include_str!("templates/README.md.tmpl");
const GITIGNORE_TEMPLATE: &str = include_str!("templates/gitignore.tmpl");

/// Options for project scaffolding.
#[derive(Clone, Debug)]
pub struct ScaffoldOptions {
    pub name: String,
    pub destination: PathBuf,
}

/// Scaffold a new mohaus project.
///
/// # Errors
///
/// Returns an error when the destination is invalid, not empty, or files cannot
/// be written.
pub fn scaffold_project(options: &ScaffoldOptions) -> Result<()> {
    let package = PackageName::parse(options.name.clone())?;
    let import_name = package.import_name();
    ensure_destination(&options.destination)?;

    let replacements = [
        ("{{project_name}}", package.as_str().to_string()),
        ("{{import_name}}", import_name.clone()),
        ("{{mojo_version}}", DEFAULT_MOJO_VERSION.to_string()),
    ];

    write_template(
        &options.destination.join("pyproject.toml"),
        PYPROJECT_TEMPLATE,
        &replacements,
    )?;
    write_template(
        &options.destination.join("src").join("lib.mojo"),
        LIB_MOJO_TEMPLATE,
        &replacements,
    )?;
    write_template(
        &options
            .destination
            .join("python")
            .join(&import_name)
            .join("__init__.py"),
        PY_INIT_TEMPLATE,
        &replacements,
    )?;
    write_file(
        &options
            .destination
            .join("python")
            .join(&import_name)
            .join("py.typed"),
        b"",
    )?;
    write_template(
        &options.destination.join("README.md"),
        README_TEMPLATE,
        &replacements,
    )?;
    write_template(
        &options.destination.join(".gitignore"),
        GITIGNORE_TEMPLATE,
        &replacements,
    )?;
    write_file(
        &options.destination.join(".mojo-version"),
        DEFAULT_MOJO_VERSION.as_bytes(),
    )?;
    Ok(())
}

fn ensure_destination(destination: &Path) -> Result<()> {
    if destination.exists() {
        if !destination.is_dir() {
            return Err(MohausError::InvalidProject {
                message: format!(
                    "destination exists and is not a directory: {}",
                    destination.display()
                ),
            });
        }
        let mut entries = fs::read_dir(destination).map_err(|source| MohausError::ReadFile {
            path: destination.to_path_buf(),
            source,
        })?;
        if entries
            .next()
            .transpose()
            .map_err(|source| MohausError::ReadFile {
                path: destination.to_path_buf(),
                source,
            })?
            .is_some()
        {
            return Err(MohausError::InvalidProject {
                message: format!("destination is not empty: {}", destination.display()),
            });
        }
        return Ok(());
    }
    fs::create_dir_all(destination).map_err(|source| MohausError::CreateDir {
        path: destination.to_path_buf(),
        source,
    })
}

fn write_template(path: &Path, template: &str, replacements: &[(&str, String)]) -> Result<()> {
    let mut rendered = template.to_string();
    for (needle, replacement) in replacements {
        rendered = rendered.replace(needle, replacement);
    }
    write_file(path, rendered.as_bytes())
}
