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
const LICENSE_TEMPLATE: &str = include_str!("templates/LICENSE.tmpl");

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
    write_template(
        &options.destination.join("LICENSE"),
        LICENSE_TEMPLATE,
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::fs;

    use mohaus_core::DEFAULT_MOJO_VERSION;
    use mohaus_core::config::ProjectConfig;
    use mohaus_core::wheel::metadata_text;
    use tempfile::TempDir;

    use crate::{ScaffoldOptions, scaffold_project};

    #[test]
    fn scaffold_round_trips_into_project_config() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("acme");
        scaffold_project(&ScaffoldOptions {
            name: "acme".to_string(),
            destination: destination.clone(),
        })
        .unwrap();

        let config = ProjectConfig::load(&destination).unwrap();
        assert_eq!(config.package.as_str(), "acme");
        assert_eq!(config.modules.len(), 1);
        assert_eq!(config.modules[0].name.as_str(), "acme._native");
        assert!(config.generate_stub);
        assert!(destination.join("LICENSE").is_file());
        assert_eq!(
            fs::read_to_string(destination.join(".mojo-version")).unwrap(),
            DEFAULT_MOJO_VERSION
        );
        let pyproject = fs::read_to_string(destination.join("pyproject.toml")).unwrap();
        assert!(pyproject.contains(&format!("\"mojo=={DEFAULT_MOJO_VERSION}\"")));
        assert!(!pyproject.contains("mojo-src = \"src\""));
        assert!(!pyproject.contains("python-src = \"python\""));
        let gitignore = fs::read_to_string(destination.join(".gitignore")).unwrap();
        assert!(gitignore.contains("/benches/\n"));
        assert!(gitignore.contains("/vendor/\n"));
    }

    #[test]
    fn scaffold_metadata_is_publishable_shape() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("acme");
        scaffold_project(&ScaffoldOptions {
            name: "acme".to_string(),
            destination: destination.clone(),
        })
        .unwrap();

        let config = ProjectConfig::load(&destination).unwrap();
        let metadata = metadata_text(&config, false);
        assert!(metadata.contains("Metadata-Version: 2.4"));
        assert!(metadata.contains("Name: acme"));
        assert!(metadata.contains("License-Expression: Apache-2.0"));
        assert!(metadata.contains("License-File: LICENSE"));
        assert!(metadata.contains("Description-Content-Type: text/markdown"));
        assert!(metadata.contains("# acme"));
    }
}
