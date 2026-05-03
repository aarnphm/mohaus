use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::DEFAULT_MOJO_VERSION;
use crate::error::{MohausError, Result};

/// Validated Python distribution name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageName(String);

impl PackageName {
    /// Parse a Python distribution name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty or contains unsupported
    /// characters.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() {
            return Err(MohausError::InvalidPackageName {
                value,
                message: "package name cannot be empty".to_string(),
            });
        }
        if !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        {
            return Err(MohausError::InvalidPackageName {
                value,
                message: "use only ASCII letters, digits, hyphen, underscore, or dot".to_string(),
            });
        }
        Ok(Self(value))
    }

    /// Original project/distribution spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Import package name derived from the distribution name.
    pub fn import_name(&self) -> String {
        self.0.replace(['-', '.'], "_")
    }

    /// Escaped distribution name for wheel and dist-info paths.
    pub fn escaped(&self) -> String {
        normalize_distribution_component(&self.0)
    }
}

/// Validated dotted Python extension module name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleName(String);

impl ModuleName {
    /// Parse a dotted Python module name.
    ///
    /// # Errors
    ///
    /// Returns an error when any dotted segment is not a Python identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let mut parts = value.split('.');
        if value.is_empty() || parts.clone().any(str::is_empty) {
            return Err(MohausError::InvalidModuleName {
                value,
                message: "module name must be dotted Python identifiers".to_string(),
            });
        }
        if !parts.all(is_python_identifier) {
            return Err(MohausError::InvalidModuleName {
                value,
                message: "each module segment must be a Python identifier".to_string(),
            });
        }
        Ok(Self(value))
    }

    /// Dotted module name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Extension leaf, used for `PyInit_<leaf>` and the native filename.
    pub fn leaf(&self) -> &str {
        self.0
            .rsplit('.')
            .next()
            .map_or(self.0.as_str(), |leaf| leaf)
    }

    /// Package-relative shared library path without the extension suffix.
    pub fn relative_path_without_suffix(&self) -> PathBuf {
        let mut path = PathBuf::new();
        for part in self.0.split('.') {
            path.push(part);
        }
        path
    }
}

/// Validated Mojo package/toolchain version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MojoVersion(String);

impl MojoVersion {
    /// Parse a Mojo version pin.
    ///
    /// # Errors
    ///
    /// Returns an error when the version is empty or contains unsupported
    /// characters.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(MohausError::InvalidMojoVersion {
                value,
                message: "version cannot be empty".to_string(),
            });
        }
        if !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+'))
        {
            return Err(MohausError::InvalidMojoVersion {
                value: trimmed.to_string(),
                message: "version must be an ASCII package/toolchain version".to_string(),
            });
        }
        Ok(Self(trimmed.to_string()))
    }

    /// Original version pin.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Normalized version used for comparing package pins to `mojo --version`.
    pub fn normalized(&self) -> String {
        normalize_mojo_version_token(&self.0)
    }
}

/// Parsed mohaus project configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfig {
    pub project_dir: PathBuf,
    pub package: PackageName,
    pub version: String,
    pub requires_python: String,
    pub mojo_version: MojoVersion,
    pub mojo_src: PathBuf,
    pub python_src: PathBuf,
    pub modules: Vec<MojoModule>,
    pub strip: bool,
    pub mojo_flags: Vec<String>,
    pub mojo_include_paths: Vec<PathBuf>,
}

impl ProjectConfig {
    /// Load `pyproject.toml` and `.mojo-version` from a project directory.
    ///
    /// # Errors
    ///
    /// Returns an error when project files cannot be read, TOML is invalid, or
    /// configured names/versions fail validation.
    pub fn load(project_dir: impl AsRef<Path>) -> Result<Self> {
        let project_dir = project_dir.as_ref().to_path_buf();
        let pyproject_path = project_dir.join("pyproject.toml");
        let pyproject_text =
            fs::read_to_string(&pyproject_path).map_err(|source| MohausError::ReadFile {
                path: pyproject_path.clone(),
                source,
            })?;
        let raw: RawPyProject =
            toml::from_str(&pyproject_text).map_err(|source| MohausError::InvalidToml {
                path: pyproject_path,
                source,
            })?;

        let package = PackageName::parse(raw.project.name)?;
        let version = raw.project.version;
        let requires_python = raw
            .project
            .requires_python
            .unwrap_or_else(|| ">=3.11".to_string());

        let mojo_version_path = project_dir.join(".mojo-version");
        let mojo_version_text = match fs::read_to_string(&mojo_version_path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                DEFAULT_MOJO_VERSION.to_string()
            }
            Err(source) => {
                return Err(MohausError::ReadFile {
                    path: mojo_version_path,
                    source,
                });
            }
        };
        let mojo_version = MojoVersion::parse(mojo_version_text.trim())?;

        let tool = raw.tool.and_then(|tool| tool.mohaus).unwrap_or_default();
        let mojo_src = tool.mojo_src.unwrap_or_else(|| PathBuf::from("src"));
        let python_src = tool.python_src.unwrap_or_else(|| PathBuf::from("python"));
        let strip = tool.strip.unwrap_or(true);
        let mojo_flags = tool.mojo_flags.unwrap_or_default();
        let mojo_include_paths = tool.mojo_include_paths.unwrap_or_default();

        let modules = if let Some(raw_modules) = tool.modules {
            raw_modules
                .into_iter()
                .map(|module| {
                    Ok(MojoModule {
                        name: ModuleName::parse(module.name)?,
                        entry: module.entry,
                    })
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            let module_name = tool
                .module_name
                .unwrap_or_else(|| format!("{}._native", package.import_name()));
            vec![MojoModule {
                name: ModuleName::parse(module_name)?,
                entry: mojo_src.join("lib.mojo"),
            }]
        };

        Ok(Self {
            project_dir,
            package,
            version,
            requires_python,
            mojo_version,
            mojo_src,
            python_src,
            modules,
            strip,
            mojo_flags,
            mojo_include_paths,
        })
    }

    /// Return the Python source root as an absolute path.
    pub fn python_source_root(&self) -> PathBuf {
        self.project_dir.join(&self.python_src)
    }

    /// Return the Mojo source root as an absolute path.
    pub fn mojo_source_root(&self) -> PathBuf {
        self.project_dir.join(&self.mojo_src)
    }

    /// Return the package dist-info directory name.
    pub fn dist_info_dir(&self) -> String {
        format!("{}-{}.dist-info", self.package.escaped(), self.version)
    }
}

/// A Mojo extension module entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MojoModule {
    pub name: ModuleName,
    pub entry: PathBuf,
}

#[derive(Debug, Deserialize)]
struct RawPyProject {
    project: RawProject,
    tool: Option<RawTool>,
}

#[derive(Debug, Deserialize)]
struct RawProject {
    name: String,
    version: String,
    #[serde(rename = "requires-python")]
    requires_python: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTool {
    mohaus: Option<RawMohaus>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawMohaus {
    mojo_src: Option<PathBuf>,
    python_src: Option<PathBuf>,
    module_name: Option<String>,
    strip: Option<bool>,
    mojo_flags: Option<Vec<String>>,
    mojo_include_paths: Option<Vec<PathBuf>>,
    modules: Option<Vec<RawModule>>,
}

#[derive(Debug, Deserialize)]
struct RawModule {
    name: String,
    entry: PathBuf,
}

fn is_python_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn normalize_distribution_component(value: &str) -> String {
    let mut output = String::new();
    let mut last_was_separator = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            output.push('_');
            last_was_separator = true;
        }
    }
    output.trim_matches('_').to_string()
}

/// Normalize either a Mojo Python package pin or `mojo --version` token.
pub fn normalize_mojo_version_token(value: &str) -> String {
    let token = value
        .split_whitespace()
        .find(|part| part.chars().any(|ch| ch.is_ascii_digit()))
        .unwrap_or(value);
    let token = token
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .trim_start_matches('v');
    let mut parts = token.split('.').collect::<Vec<_>>();
    if parts.len() >= 4 && parts.first() == Some(&"0") {
        parts.remove(0);
    }
    parts.join(".")
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use crate::config::{ModuleName, MojoVersion, PackageName};

    #[test]
    fn package_name_derives_import_name() {
        let package = PackageName::parse("my-project.lib").unwrap();
        assert_eq!(package.import_name(), "my_project_lib");
        assert_eq!(package.escaped(), "my_project_lib");
    }

    #[test]
    fn module_name_validates_segments() {
        assert!(ModuleName::parse("pkg._native").is_ok());
        assert!(ModuleName::parse("pkg.2native").is_err());
    }

    #[test]
    fn mojo_version_normalizes_python_package_pin() {
        let version = MojoVersion::parse("0.26.2.0").unwrap();
        assert_eq!(version.normalized(), "26.2.0");
    }
}
