//! Project scaffolding for mohaus.

use std::fs;
use std::path::{Path, PathBuf};

use mohaus_core::config::{MojoVersion, PackageName};
use mohaus_core::error::{MohausError, Result};
use mohaus_core::wheel::write_file;

const PYPROJECT_TEMPLATE: &str = include_str!("templates/pyproject.toml.tmpl");
const FLAKE_NIX_TEMPLATE: &str = include_str!("templates/flake.nix.tmpl");
const LIB_MOJO_TEMPLATE: &str = include_str!("templates/lib.mojo.tmpl");
const PY_INIT_TEMPLATE: &str = include_str!("templates/__init__.py.tmpl");
const README_TEMPLATE: &str = include_str!("templates/README.md.tmpl");
const GITIGNORE_TEMPLATE: &str = include_str!("templates/gitignore.tmpl");
const GITATTRIBUTES_TEMPLATE: &str = include_str!("templates/gitattributes.tmpl");
const LICENSE_TEMPLATE: &str = include_str!("templates/LICENSE.tmpl");

/// Options for project scaffolding.
#[derive(Clone, Debug)]
pub struct ScaffoldOptions {
    pub name: String,
    pub destination: PathBuf,
    pub mojo_version: Option<MojoVersion>,
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
    ];

    write_template(
        &options.destination.join("pyproject.toml"),
        PYPROJECT_TEMPLATE,
        &replacements,
    )?;
    write_template(
        &options.destination.join("flake.nix"),
        FLAKE_NIX_TEMPLATE,
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
        &options.destination.join(".gitattributes"),
        GITATTRIBUTES_TEMPLATE,
        &replacements,
    )?;
    write_template(
        &options.destination.join("LICENSE"),
        LICENSE_TEMPLATE,
        &replacements,
    )?;
    if let Some(mojo_version) = options.mojo_version.as_ref() {
        write_file(
            &options.destination.join(".mojo-version"),
            mojo_version.as_str().as_bytes(),
        )?;
    }
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

    use mohaus_core::config::{MojoVersion, ProjectConfig};
    use mohaus_core::wheel::metadata_text;
    use tempfile::TempDir;

    use crate::{ScaffoldOptions, scaffold_project};

    fn scaffold_options(name: &str, destination: std::path::PathBuf) -> ScaffoldOptions {
        ScaffoldOptions {
            name: name.to_string(),
            destination,
            mojo_version: None,
        }
    }

    #[test]
    fn scaffold_round_trips_into_project_config() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("acme");
        scaffold_project(&scaffold_options("acme", destination.clone())).unwrap();

        let config = ProjectConfig::load(&destination).unwrap();
        assert_eq!(config.package.as_str(), "acme");
        assert_eq!(config.modules.len(), 1);
        assert_eq!(config.modules[0].name.as_str(), "acme._native");
        assert!(config.generate_stub);
        assert!(destination.join("LICENSE").is_file());
        assert!(!destination.join(".mojo-version").exists());
        let pyproject = fs::read_to_string(destination.join("pyproject.toml")).unwrap();
        assert!(pyproject.contains("\"modular\","));
        assert!(!pyproject.contains("\"mojo=="));
        assert!(!pyproject.contains("\"mojo-compiler=="));
        assert!(!pyproject.contains("\"mojo-compiler-mojo-libs=="));
        assert!(!pyproject.contains("\"mojo-lldb-libs=="));
        assert!(!pyproject.contains("mojo-src = \"src\""));
        assert!(!pyproject.contains("python-src = \"python\""));
        assert!(pyproject.contains("extend-include = [\"*.ipynb\"]"));
        assert!(pyproject.contains("[tool.ty.rules]\nall = \"error\""));
        let flake = fs::read_to_string(destination.join("flake.nix")).unwrap();
        assert!(flake.contains(
            "description = \"acme: mixed Python and Mojo package scaffolded by mohaus\";"
        ));
        assert!(flake.contains("git-hooks-nix.url = \"github:cachix/git-hooks.nix\";"));
        assert!(flake.contains("mohaus.url = \"github:aarnphm/mohaus\";"));
        assert!(flake.contains("mohaus develop"));
        assert!(flake.contains("pre-commit = git-hooks-nix.lib.${system}.run"));
        assert!(flake.contains("uvx ty check"));
        assert!(!flake.contains("oxfmt"));
        let readme = fs::read_to_string(destination.join("README.md")).unwrap();
        assert!(readme.contains("https://whl.modular.com/nightly/simple/"));
        let gitignore = fs::read_to_string(destination.join(".gitignore")).unwrap();
        assert!(!gitignore.contains("/benches/\n"));
        assert!(gitignore.contains("/vendor/\n"));
        let gitattributes = fs::read_to_string(destination.join(".gitattributes")).unwrap();
        assert!(gitattributes.contains("/vendor/** linguist-vendored\n"));
    }

    #[test]
    fn scaffold_writes_optional_mojo_version_pin() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("acme");
        let mut options = scaffold_options("acme", destination.clone());
        options.mojo_version = Some(MojoVersion::parse("1.0.0b2.dev2026050805").unwrap());
        scaffold_project(&options).unwrap();

        assert_eq!(
            fs::read_to_string(destination.join(".mojo-version")).unwrap(),
            "1.0.0b2.dev2026050805"
        );
        let pyproject = fs::read_to_string(destination.join("pyproject.toml")).unwrap();
        assert!(pyproject.contains("\"modular\","));
        assert!(!pyproject.contains("1.0.0b2.dev2026050805"));
    }

    #[test]
    fn scaffold_metadata_is_publishable_shape() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("acme");
        scaffold_project(&scaffold_options("acme", destination.clone())).unwrap();

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
