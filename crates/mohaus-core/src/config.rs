use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{MohausError, Result};

const RESERVED_MOJO_FLAG_PREFIXES: &[&str] = &["-o", "--output", "-I", "--include", "--emit"];

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

/// Author or maintainer attribution.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AuthorRecord {
    pub name: Option<String>,
    pub email: Option<String>,
}

impl AuthorRecord {
    fn is_empty(&self) -> bool {
        self.name.is_none() && self.email.is_none()
    }

    /// Format as `Name <email>` for METADATA Author/Maintainer fields.
    pub fn formatted(&self) -> Option<String> {
        match (self.name.as_ref(), self.email.as_ref()) {
            (Some(name), Some(email)) => Some(format!("{name} <{email}>")),
            (Some(name), None) => Some(name.clone()),
            (None, Some(email)) => Some(email.clone()),
            (None, None) => None,
        }
    }
}

/// PEP 621 license content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum License {
    /// SPDX expression or free text identifier (PEP 639 `license`).
    Expression(String),
    /// Inline license body.
    Text(String),
    /// License body sourced from a file relative to the project root.
    File { path: PathBuf, body: String },
}

/// PEP 621 readme content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Readme {
    pub content_type: String,
    pub body: String,
}

/// Project metadata parsed from `[project]`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectMetadata {
    pub summary: Option<String>,
    pub readme: Option<Readme>,
    pub license: Option<License>,
    pub license_files: Vec<PathBuf>,
    pub authors: Vec<AuthorRecord>,
    pub maintainers: Vec<AuthorRecord>,
    pub keywords: Vec<String>,
    pub classifiers: Vec<String>,
    pub urls: Vec<(String, String)>,
    pub dependencies: Vec<String>,
    pub optional_dependencies: Vec<(String, Vec<String>)>,
    pub scripts: Vec<(String, String)>,
    pub gui_scripts: Vec<(String, String)>,
    pub entry_points: Vec<(String, Vec<(String, String)>)>,
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
    pub metadata: ProjectMetadata,
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

        let package = PackageName::parse(raw.project.name.clone())?;
        let version = raw.project.version.clone();
        let requires_python = raw
            .project
            .requires_python
            .clone()
            .unwrap_or_else(|| ">=3.11".to_string());

        let metadata = build_project_metadata(&project_dir, &raw.project)?;

        let mojo_version_path = project_dir.join(".mojo-version");
        let mojo_version_text =
            fs::read_to_string(&mojo_version_path).map_err(|source| match source.kind() {
                std::io::ErrorKind::NotFound => MohausError::InvalidProject {
                    message: format!(
                        ".mojo-version is required at {} but is missing; pin a Mojo toolchain with `echo <version> > .mojo-version`",
                        mojo_version_path.display()
                    ),
                },
                _ => MohausError::ReadFile {
                    path: mojo_version_path.clone(),
                    source,
                },
            })?;
        let mojo_version = MojoVersion::parse(mojo_version_text.trim())?;

        let tool = raw.tool.and_then(|tool| tool.mohaus).unwrap_or_default();
        let mojo_src = tool.mojo_src.unwrap_or_else(|| PathBuf::from("src"));
        let python_src = tool.python_src.unwrap_or_else(|| PathBuf::from("python"));
        let strip = tool.strip.unwrap_or(true);
        let mojo_flags = tool.mojo_flags.unwrap_or_default();
        validate_mojo_flags(&mojo_flags)?;
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
            metadata,
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

#[derive(Clone, Debug, Deserialize)]
struct RawProject {
    name: String,
    version: String,
    #[serde(rename = "requires-python")]
    requires_python: Option<String>,
    description: Option<String>,
    readme: Option<RawReadme>,
    license: Option<RawLicense>,
    #[serde(rename = "license-files")]
    license_files: Option<Vec<PathBuf>>,
    authors: Option<Vec<RawPersonOrString>>,
    maintainers: Option<Vec<RawPersonOrString>>,
    keywords: Option<Vec<String>>,
    classifiers: Option<Vec<String>>,
    urls: Option<std::collections::BTreeMap<String, String>>,
    dependencies: Option<Vec<String>>,
    #[serde(rename = "optional-dependencies")]
    optional_dependencies: Option<std::collections::BTreeMap<String, Vec<String>>>,
    scripts: Option<std::collections::BTreeMap<String, String>>,
    #[serde(rename = "gui-scripts")]
    gui_scripts: Option<std::collections::BTreeMap<String, String>>,
    #[serde(rename = "entry-points")]
    entry_points:
        Option<std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum RawReadme {
    Path(String),
    Inline {
        file: Option<PathBuf>,
        text: Option<String>,
        #[serde(rename = "content-type")]
        content_type: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum RawLicense {
    Expression(String),
    Inline {
        file: Option<PathBuf>,
        text: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum RawPersonOrString {
    Inline {
        name: Option<String>,
        email: Option<String>,
    },
    Bare(String),
}

fn build_project_metadata(project_dir: &Path, raw: &RawProject) -> Result<ProjectMetadata> {
    let readme = match raw.readme.as_ref() {
        Some(RawReadme::Path(path)) => Some(load_readme_path(project_dir, Path::new(path))?),
        Some(RawReadme::Inline {
            file: Some(file),
            content_type,
            ..
        }) => {
            let mut readme = load_readme_path(project_dir, file)?;
            if let Some(ct) = content_type {
                readme.content_type = ct.clone();
            }
            Some(readme)
        }
        Some(RawReadme::Inline {
            text: Some(text),
            content_type,
            ..
        }) => Some(Readme {
            content_type: content_type
                .clone()
                .unwrap_or_else(|| "text/markdown".to_string()),
            body: text.clone(),
        }),
        Some(RawReadme::Inline {
            file: None,
            text: None,
            ..
        }) => None,
        None => None,
    };

    let license = match raw.license.as_ref() {
        Some(RawLicense::Expression(value)) => Some(License::Expression(value.clone())),
        Some(RawLicense::Inline {
            file: Some(file), ..
        }) => {
            let absolute = project_dir.join(file);
            let body = fs::read_to_string(&absolute).map_err(|source| MohausError::ReadFile {
                path: absolute.clone(),
                source,
            })?;
            Some(License::File {
                path: file.clone(),
                body,
            })
        }
        Some(RawLicense::Inline {
            text: Some(text), ..
        }) => Some(License::Text(text.clone())),
        Some(RawLicense::Inline {
            file: None,
            text: None,
        }) => None,
        None => None,
    };

    let authors = raw
        .authors
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(person_or_string_to_record)
        .filter(|record| !record.is_empty())
        .collect();
    let maintainers = raw
        .maintainers
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(person_or_string_to_record)
        .filter(|record| !record.is_empty())
        .collect();

    let urls = raw
        .urls
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>();
    let optional_dependencies = raw
        .optional_dependencies
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>();
    let scripts = raw
        .scripts
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>();
    let gui_scripts = raw
        .gui_scripts
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>();
    let entry_points = raw
        .entry_points
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|(group, table)| (group, table.into_iter().collect::<Vec<_>>()))
        .collect::<Vec<_>>();

    Ok(ProjectMetadata {
        summary: raw.description.clone(),
        readme,
        license,
        license_files: raw.license_files.clone().unwrap_or_default(),
        authors,
        maintainers,
        keywords: raw.keywords.clone().unwrap_or_default(),
        classifiers: raw.classifiers.clone().unwrap_or_default(),
        urls,
        dependencies: raw.dependencies.clone().unwrap_or_default(),
        optional_dependencies,
        scripts,
        gui_scripts,
        entry_points,
    })
}

fn person_or_string_to_record(value: RawPersonOrString) -> AuthorRecord {
    match value {
        RawPersonOrString::Inline { name, email } => AuthorRecord { name, email },
        RawPersonOrString::Bare(value) => AuthorRecord {
            name: Some(value),
            email: None,
        },
    }
}

fn load_readme_path(project_dir: &Path, path: &Path) -> Result<Readme> {
    let absolute = project_dir.join(path);
    let body = fs::read_to_string(&absolute).map_err(|source| MohausError::ReadFile {
        path: absolute.clone(),
        source,
    })?;
    let content_type = readme_content_type_for_path(path);
    Ok(Readme { content_type, body })
}

fn readme_content_type_for_path(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("rst") => "text/x-rst".to_string(),
        Some("txt") | None => "text/plain".to_string(),
        _ => "text/markdown".to_string(),
    }
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

fn validate_mojo_flags(flags: &[String]) -> Result<()> {
    for flag in flags {
        let trimmed = flag.trim();
        for reserved in RESERVED_MOJO_FLAG_PREFIXES {
            let matches = trimmed == *reserved
                || trimmed.starts_with(&format!("{reserved}="))
                || (reserved.starts_with("--") && trimmed.starts_with(&format!("{reserved} ")));
            if matches {
                return Err(MohausError::InvalidProject {
                    message: format!(
                        "tool.mohaus.mojo-flags contains reserved flag `{trimmed}`; mohaus owns -o, --output, -I, --include, and --emit"
                    ),
                });
            }
        }
    }
    Ok(())
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
    use std::fs;

    use tempfile::TempDir;

    use crate::config::{ModuleName, MojoVersion, PackageName, ProjectConfig};
    use crate::error::MohausError;

    fn write_pyproject(root: &std::path::Path, body: &str) {
        fs::write(root.join("pyproject.toml"), body).unwrap();
    }

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

    #[test]
    fn mojo_version_preserves_nightly_prerelease_pin() {
        let version = MojoVersion::parse("1.0.0b2.dev2026050306").unwrap();
        assert_eq!(version.normalized(), "1.0.0b2.dev2026050306");
    }

    #[test]
    fn missing_mojo_version_file_is_an_actionable_error() {
        let root = TempDir::new().unwrap();
        write_pyproject(
            root.path(),
            r#"
[project]
name = "demo"
version = "0.1.0"

[tool.mohaus]
module-name = "demo._native"
"#,
        );
        let error = ProjectConfig::load(root.path()).unwrap_err();
        match error {
            MohausError::InvalidProject { message } => {
                assert!(
                    message.contains(".mojo-version is required"),
                    "unexpected error message: {message}"
                );
            }
            other => panic!("expected InvalidProject, got {other:?}"),
        }
    }

    #[test]
    fn reserved_mojo_flags_are_rejected() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join(".mojo-version"), "0.26.2.0").unwrap();
        write_pyproject(
            root.path(),
            r#"
[project]
name = "demo"
version = "0.1.0"

[tool.mohaus]
module-name = "demo._native"
mojo-flags = ["-o", "fast.so"]
"#,
        );
        let error = ProjectConfig::load(root.path()).unwrap_err();
        match error {
            MohausError::InvalidProject { message } => {
                assert!(message.contains("reserved flag"), "got: {message}");
            }
            other => panic!("expected InvalidProject, got {other:?}"),
        }
    }

    #[test]
    fn allows_safe_mojo_flags() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join(".mojo-version"), "0.26.2.0").unwrap();
        write_pyproject(
            root.path(),
            r#"
[project]
name = "demo"
version = "0.1.0"

[tool.mohaus]
module-name = "demo._native"
mojo-flags = ["-O3", "-debug-level=full"]
"#,
        );
        let config = ProjectConfig::load(root.path()).unwrap();
        assert_eq!(
            config.mojo_flags,
            vec!["-O3".to_string(), "-debug-level=full".to_string()]
        );
    }
}
