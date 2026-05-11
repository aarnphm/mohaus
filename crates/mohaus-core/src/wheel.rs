use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;
use zip::CompressionMethod;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::config::{License, ProjectConfig};
use crate::error::{MohausError, Result};

/// Write METADATA content for a project, conforming to PEP 643 / Metadata 2.4.
pub fn metadata_text(config: &ProjectConfig, editable: bool) -> String {
    let metadata = &config.metadata;
    let mut text = String::new();
    text.push_str("Metadata-Version: 2.4\n");
    text.push_str(&format!("Name: {}\n", config.package.as_str()));
    text.push_str(&format!("Version: {}\n", config.version));
    if let Some(summary) = metadata.summary.as_ref() {
        text.push_str(&format!("Summary: {}\n", summary.replace('\n', " ")));
    }
    text.push_str(&format!("Requires-Python: {}\n", config.requires_python));
    if let Some(license) = metadata.license.as_ref() {
        match license {
            License::Expression(value) => {
                text.push_str(&format!("License-Expression: {value}\n"));
            }
            License::Text(_) | License::File { .. } => {
                text.push_str("License: see License section\n");
            }
        }
    }
    for license_file in &metadata.license_files {
        text.push_str(&format!("License-File: {}\n", license_file.display()));
    }
    for keyword in folded_keywords(&metadata.keywords) {
        text.push_str(&format!("Keywords: {keyword}\n"));
    }
    for classifier in &metadata.classifiers {
        text.push_str(&format!("Classifier: {classifier}\n"));
    }
    for author in &metadata.authors {
        if let Some(value) = author.formatted() {
            text.push_str(&format!("Author: {value}\n"));
        }
    }
    for author in &metadata.maintainers {
        if let Some(value) = author.formatted() {
            text.push_str(&format!("Maintainer: {value}\n"));
        }
    }
    for (label, url) in &metadata.urls {
        text.push_str(&format!("Project-URL: {label}, {url}\n"));
    }
    for dep in &metadata.dependencies {
        text.push_str(&format!("Requires-Dist: {dep}\n"));
    }
    for (extra, deps) in &metadata.optional_dependencies {
        text.push_str(&format!("Provides-Extra: {extra}\n"));
        for dep in deps {
            if dep.contains(';') {
                text.push_str(&format!("Requires-Dist: {dep} and extra == {extra:?}\n"));
            } else {
                text.push_str(&format!("Requires-Dist: {dep}; extra == {extra:?}\n"));
            }
        }
    }
    if editable {
        text.push_str("Requires-Dist: mohaus>=0.1,<0.2\n");
        if let Some(version) = editable_mojo_pin(config) {
            text.push_str(&format!("Requires-Dist: mojo=={version}\n"));
        }
    }
    if let Some(readme) = metadata.readme.as_ref() {
        text.push_str(&format!(
            "Description-Content-Type: {}\n",
            readme.content_type
        ));
    }
    text.push('\n');
    if let Some(license_body) = inline_license_body(metadata.license.as_ref()) {
        text.push_str("License Section\n===============\n\n");
        text.push_str(&license_body);
        if !license_body.ends_with('\n') {
            text.push('\n');
        }
        text.push('\n');
    }
    if let Some(readme) = metadata.readme.as_ref() {
        text.push_str(&readme.body);
        if !readme.body.ends_with('\n') {
            text.push('\n');
        }
    }
    text
}

fn inline_license_body(license: Option<&License>) -> Option<String> {
    match license? {
        License::Text(value) => Some(value.clone()),
        License::File { body, .. } => Some(body.clone()),
        License::Expression(_) => None,
    }
}

fn folded_keywords(keywords: &[String]) -> Vec<String> {
    if keywords.is_empty() {
        return Vec::new();
    }
    vec![keywords.join(",")]
}

fn editable_mojo_pin(config: &ProjectConfig) -> Option<&str> {
    let version = config.mojo_version.as_ref()?.as_str();
    if version.contains("dev") || version.contains("nightly") {
        return None;
    }
    Some(version)
}

/// Write WHEEL content.
pub fn wheel_text(tag: &str, root_is_purelib: bool) -> String {
    format!(
        "Wheel-Version: 1.0\nGenerator: mohaus 0.1.0\nRoot-Is-Purelib: {}\nTag: {}\n\n",
        if root_is_purelib { "true" } else { "false" },
        tag
    )
}

/// Prepare a dist-info directory without RECORD.
///
/// # Errors
///
/// Returns an error when the dist-info directory or metadata files cannot be
/// written.
pub fn write_dist_info(
    root: &Path,
    config: &ProjectConfig,
    tag: &str,
    root_is_purelib: bool,
    editable: bool,
) -> Result<PathBuf> {
    let dist_info = root.join(config.dist_info_dir());
    fs::create_dir_all(&dist_info).map_err(|source| MohausError::CreateDir {
        path: dist_info.clone(),
        source,
    })?;
    write_file(
        &dist_info.join("METADATA"),
        metadata_text(config, editable).as_bytes(),
    )?;
    write_file(
        &dist_info.join("WHEEL"),
        wheel_text(tag, root_is_purelib).as_bytes(),
    )?;
    if let Some(entry_points) = entry_points_text(config) {
        write_file(&dist_info.join("entry_points.txt"), entry_points.as_bytes())?;
    }
    write_license_files(&dist_info, config)?;
    Ok(dist_info)
}

fn entry_points_text(config: &ProjectConfig) -> Option<String> {
    let metadata = &config.metadata;
    if metadata.scripts.is_empty()
        && metadata.gui_scripts.is_empty()
        && metadata.entry_points.is_empty()
    {
        return None;
    }
    let mut text = String::new();
    if !metadata.scripts.is_empty() {
        text.push_str("[console_scripts]\n");
        for (name, target) in &metadata.scripts {
            text.push_str(&format!("{name} = {target}\n"));
        }
        text.push('\n');
    }
    if !metadata.gui_scripts.is_empty() {
        text.push_str("[gui_scripts]\n");
        for (name, target) in &metadata.gui_scripts {
            text.push_str(&format!("{name} = {target}\n"));
        }
        text.push('\n');
    }
    for (group, entries) in &metadata.entry_points {
        text.push_str(&format!("[{group}]\n"));
        for (name, target) in entries {
            text.push_str(&format!("{name} = {target}\n"));
        }
        text.push('\n');
    }
    Some(text)
}

fn write_license_files(dist_info: &Path, config: &ProjectConfig) -> Result<()> {
    if config.metadata.license_files.is_empty() {
        return Ok(());
    }
    let target_root = dist_info.join("licenses");
    for relative in &config.metadata.license_files {
        let absolute = config.project_dir.join(relative);
        if !absolute.is_file() {
            return Err(MohausError::WheelMetadata {
                message: format!(
                    "[project] license-files references missing file: {}",
                    absolute.display()
                ),
            });
        }
        let dest = target_root.join(relative);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|source| MohausError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(&absolute, &dest).map_err(|source| MohausError::CopyFile {
            source_path: absolute.clone(),
            dest_path: dest,
            source,
        })?;
    }
    Ok(())
}

/// Reuse a previously prepared dist-info if PEP 517 metadata_directory points
/// at one. Returns Ok(true) when a dist-info was found and copied, Ok(false)
/// when the caller should regenerate it.
///
/// # Errors
///
/// Returns an error when the prepared dist-info cannot be read, copied, or
/// names a different package than the active project.
pub fn copy_prepared_dist_info(
    metadata_dir: &Path,
    staged_root: &Path,
    config: &ProjectConfig,
) -> Result<bool> {
    let expected = config.dist_info_dir();
    let candidate = metadata_dir.join(&expected);
    if !candidate.is_dir() {
        let entries = match fs::read_dir(metadata_dir) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => {
                return Err(MohausError::ReadFile {
                    path: metadata_dir.to_path_buf(),
                    source,
                });
            }
        };
        for entry in entries {
            let entry = entry.map_err(|source| MohausError::ReadFile {
                path: metadata_dir.to_path_buf(),
                source,
            })?;
            if let Some(name) = entry.file_name().to_str()
                && name.ends_with(".dist-info")
                && entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
                && name != expected
            {
                return Err(MohausError::WheelMetadata {
                    message: format!(
                        "metadata_directory contains `{name}` but project produces `{expected}`"
                    ),
                });
            }
        }
        return Ok(false);
    }
    let staged_dist_info = staged_root.join(&expected);
    fs::create_dir_all(&staged_dist_info).map_err(|source| MohausError::CreateDir {
        path: staged_dist_info.clone(),
        source,
    })?;
    copy_dist_info_contents(&candidate, &staged_dist_info)?;
    Ok(true)
}

fn copy_dist_info_contents(source_root: &Path, dest: &Path) -> Result<()> {
    for entry in WalkDir::new(source_root) {
        let entry = entry.map_err(|source| MohausError::WalkDir {
            path: source_root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path == source_root {
            continue;
        }
        let relative =
            path.strip_prefix(source_root)
                .map_err(|source| MohausError::WheelMetadata {
                    message: format!("could not relativize prepared dist-info: {source}"),
                })?;
        if relative
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name == "RECORD")
        {
            continue;
        }
        let target = dest.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|source| MohausError::CreateDir {
                path: target,
                source,
            })?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|source| MohausError::CreateDir {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            fs::copy(path, &target).map_err(|source| MohausError::CopyFile {
                source_path: path.to_path_buf(),
                dest_path: target,
                source,
            })?;
        }
    }
    Ok(())
}

/// Write a wheel from a staged root.
///
/// # Errors
///
/// Returns an error when RECORD generation, zip creation, or file streaming
/// fails.
pub fn write_wheel_archive(
    staged_root: &Path,
    wheel_path: &Path,
    dist_info_dir: &str,
) -> Result<()> {
    let record_path = staged_root.join(dist_info_dir).join("RECORD");
    write_record(staged_root, &record_path)?;

    if let Some(parent) = wheel_path.parent() {
        fs::create_dir_all(parent).map_err(|source| MohausError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let file = fs::File::create(wheel_path).map_err(|source| MohausError::WriteFile {
        path: wheel_path.to_path_buf(),
        source,
    })?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);

    for path in sorted_files(staged_root)? {
        let relative = relative_zip_path(staged_root, &path)?;
        zip.start_file(relative, options)
            .map_err(|source| MohausError::Zip {
                path: wheel_path.to_path_buf(),
                source,
            })?;
        let mut input = fs::File::open(&path).map_err(|source| MohausError::ReadFile {
            path: path.clone(),
            source,
        })?;
        std::io::copy(&mut input, &mut zip).map_err(|source| MohausError::WriteFile {
            path: wheel_path.to_path_buf(),
            source,
        })?;
    }
    zip.finish().map_err(|source| MohausError::Zip {
        path: wheel_path.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Write bytes to a file, creating parents.
///
/// # Errors
///
/// Returns an error when the parent directory or file cannot be written.
pub fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| MohausError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut file = fs::File::create(path).map_err(|source| MohausError::WriteFile {
        path: path.to_path_buf(),
        source,
    })?;
    file.write_all(bytes)
        .map_err(|source| MohausError::WriteFile {
            path: path.to_path_buf(),
            source,
        })
}

/// Recursively copy one directory into another, skipping development artifacts.
///
/// Filters out `__pycache__`, compiled `.pyc`/`.pyo` files, in-place `.so` /
/// `.dylib` / `.pyd` extensions left over from editable builds, dotfile
/// metadata directories, and editor swap files. Production wheels must not
/// ship developer detritus.
///
/// # Errors
///
/// Returns an error when the source tree cannot be walked or copied.
pub fn copy_dir(source_root: &Path, dest_root: &Path) -> Result<()> {
    let mut walker = WalkDir::new(source_root).into_iter();
    while let Some(entry) = walker.next() {
        let entry = entry.map_err(|source| MohausError::WalkDir {
            path: source_root.to_path_buf(),
            source,
        })?;
        let source_path = entry.path();
        if source_path == source_root {
            continue;
        }
        let relative =
            source_path
                .strip_prefix(source_root)
                .map_err(|source| MohausError::WheelMetadata {
                    message: format!("could not relativize {}: {source}", source_path.display()),
                })?;
        if should_skip_staging_path(source_path, relative) {
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        let dest_path = dest_root.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest_path).map_err(|source| MohausError::CreateDir {
                path: dest_path,
                source,
            })?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent).map_err(|source| MohausError::CreateDir {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            fs::copy(source_path, &dest_path).map_err(|source| MohausError::CopyFile {
                source_path: source_path.to_path_buf(),
                dest_path,
                source,
            })?;
        }
    }
    Ok(())
}

const STAGING_SKIPPED_DIRS: &[&str] = &[
    "__pycache__",
    "__mojocache__",
    ".mohaus",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
];

const STAGING_SKIPPED_SUFFIXES: &[&str] =
    &[".pyc", ".pyo", ".so", ".dylib", ".pyd", ".swp", ".swo"];

/// Top-level metadata files that live next to the package source but must not
/// ship as runtime artifacts. They're already captured in METADATA via
/// `[project] readme` and `[project] license-files`, and pyproject is build
/// config. Only matched when they appear directly at the staged package root.
const STAGING_SKIPPED_TOPLEVEL: &[&str] = &[
    "pyproject.toml",
    "README.md",
    "README.rst",
    "README.txt",
    "README",
    "LICENSE",
    "LICENSE.md",
    "LICENSE.txt",
    "LICENCE",
    ".mojo-version",
    ".gitignore",
    ".gitattributes",
    "flake.nix",
];

fn should_skip_staging_path(path: &Path, relative: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|value| value.to_str())
        && (name.starts_with(".#") || name.ends_with('~'))
    {
        return true;
    }
    if let Some(name) = relative.to_str()
        && STAGING_SKIPPED_TOPLEVEL.contains(&name)
    {
        return true;
    }
    for component in relative.components() {
        if let Some(name) = component.as_os_str().to_str() {
            if STAGING_SKIPPED_DIRS.contains(&name) {
                return true;
            }
            if name == "py.typed" {
                continue;
            }
            for suffix in STAGING_SKIPPED_SUFFIXES {
                if name.ends_with(suffix) {
                    return true;
                }
            }
        }
    }
    false
}

fn write_record(staged_root: &Path, record_path: &Path) -> Result<()> {
    let mut lines = Vec::new();
    for path in sorted_files(staged_root)? {
        let relative = relative_zip_path(staged_root, &path)?;
        if path == record_path {
            lines.push(format!("{relative},,"));
            continue;
        }
        let bytes = fs::read(&path).map_err(|source| MohausError::ReadFile {
            path: path.clone(),
            source,
        })?;
        let digest = Sha256::digest(&bytes);
        let encoded = URL_SAFE_NO_PAD.encode(digest);
        lines.push(format!("{relative},sha256={encoded},{}", bytes.len()));
    }
    lines.sort();
    write_file(record_path, lines.join("\n").as_bytes())
}

fn sorted_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root) {
        let entry = entry.map_err(|source| MohausError::WalkDir {
            path: root.to_path_buf(),
            source,
        })?;
        if entry.file_type().is_file() {
            files.push(entry.path().to_path_buf());
        }
    }
    files.sort();
    Ok(files)
}

fn relative_zip_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|source| MohausError::WheelMetadata {
            message: format!("could not relativize {}: {source}", path.display()),
        })?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

/// Compute a sha256 hash for one file.
///
/// # Errors
///
/// Returns an error when the file cannot be opened or read.
pub fn file_hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|source| MohausError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| MohausError::ReadFile {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(URL_SAFE_NO_PAD.encode(hasher.finalize()))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::path::PathBuf;

    use crate::config::{MojoVersion, PackageName};
    use crate::wheel::metadata_text;

    fn config_with_mojo(version: &str) -> crate::config::ProjectConfig {
        crate::config::ProjectConfig {
            project_dir: PathBuf::from("/project"),
            package: PackageName::parse("demo").unwrap(),
            version: "0.1.0".to_string(),
            requires_python: ">=3.11".to_string(),
            mojo_version: Some(MojoVersion::parse(version).unwrap()),
            mojo_src: PathBuf::from("src"),
            python_src: PathBuf::from("python"),
            modules: Vec::new(),
            strip: true,
            generate_stub: true,
            mojo_flags: Vec::new(),
            mojo_include_paths: Vec::new(),
            metadata: crate::config::ProjectMetadata::default(),
            pure: false,
        }
    }

    fn config_with_full_metadata() -> crate::config::ProjectConfig {
        use crate::config::{AuthorRecord, License, ProjectMetadata, Readme};
        let metadata = ProjectMetadata {
            summary: Some("A demo".to_string()),
            readme: Some(Readme {
                content_type: "text/markdown".to_string(),
                body: "# demo\n".to_string(),
            }),
            license: Some(License::Expression("Apache-2.0".to_string())),
            license_files: Vec::new(),
            authors: vec![AuthorRecord {
                name: Some("Aaron".to_string()),
                email: Some("a@example.com".to_string()),
            }],
            maintainers: Vec::new(),
            keywords: vec!["mojo".to_string(), "python".to_string()],
            classifiers: vec!["Programming Language :: Rust".to_string()],
            urls: vec![("Source".to_string(), "https://example.com".to_string())],
            dependencies: vec!["numpy>=1".to_string()],
            optional_dependencies: vec![("dev".to_string(), vec!["pytest>=8".to_string()])],
            scripts: vec![("demo".to_string(), "demo._cli:main".to_string())],
            gui_scripts: Vec::new(),
            entry_points: Vec::new(),
        };
        crate::config::ProjectConfig {
            project_dir: PathBuf::from("/project"),
            package: PackageName::parse("demo").unwrap(),
            version: "0.1.0".to_string(),
            requires_python: ">=3.11".to_string(),
            mojo_version: Some(MojoVersion::parse("0.26.2.0").unwrap()),
            mojo_src: PathBuf::from("src"),
            python_src: PathBuf::from("python"),
            modules: Vec::new(),
            strip: true,
            generate_stub: true,
            mojo_flags: Vec::new(),
            mojo_include_paths: Vec::new(),
            metadata,
            pure: false,
        }
    }

    #[test]
    fn metadata_text_emits_full_pep_621_fields() {
        let metadata = metadata_text(&config_with_full_metadata(), false);

        assert!(metadata.contains("Metadata-Version: 2.4"));
        assert!(metadata.contains("Summary: A demo"));
        assert!(metadata.contains("License-Expression: Apache-2.0"));
        assert!(metadata.contains("Author: Aaron <a@example.com>"));
        assert!(metadata.contains("Keywords: mojo,python"));
        assert!(metadata.contains("Classifier: Programming Language :: Rust"));
        assert!(metadata.contains("Project-URL: Source, https://example.com"));
        assert!(metadata.contains("Requires-Dist: numpy>=1"));
        assert!(metadata.contains("Provides-Extra: dev"));
        assert!(metadata.contains("Requires-Dist: pytest>=8; extra == \"dev\""));
        assert!(metadata.contains("Description-Content-Type: text/markdown"));
        assert!(metadata.contains("# demo"));
    }

    #[test]
    fn entry_points_text_renders_console_scripts() {
        let config = config_with_full_metadata();
        let text = super::entry_points_text(&config).unwrap();
        assert!(text.contains("[console_scripts]"));
        assert!(text.contains("demo = demo._cli:main"));
    }

    #[test]
    fn entry_points_text_returns_none_when_empty() {
        let config = config_with_mojo("0.26.2.0");
        assert!(super::entry_points_text(&config).is_none());
    }

    #[test]
    fn editable_metadata_requires_stable_mojo_package() {
        let metadata = metadata_text(&config_with_mojo("0.26.2.0"), true);

        assert!(metadata.contains("Requires-Dist: mohaus>=0.1,<0.2\n"));
        assert!(metadata.contains("Requires-Dist: mojo==0.26.2.0\n"));
    }

    #[test]
    fn editable_metadata_does_not_require_dev_mojo_package() {
        let metadata = metadata_text(&config_with_mojo("1.0.0.dev0"), true);

        assert!(metadata.contains("Requires-Dist: mohaus>=0.1,<0.2\n"));
        assert!(!metadata.contains("Requires-Dist: mojo=="));
    }

    #[test]
    fn production_metadata_does_not_require_build_runtime_packages() {
        let metadata = metadata_text(&config_with_mojo("0.26.2.0"), false);

        assert!(!metadata.contains("Requires-Dist: mohaus"));
        assert!(!metadata.contains("Requires-Dist: mojo"));
    }

    #[test]
    fn copy_prepared_dist_info_reuses_metadata_when_present() {
        use std::fs;
        use tempfile::TempDir;

        let prepared = TempDir::new().unwrap();
        let staged = TempDir::new().unwrap();
        let config = config_with_mojo("0.26.2.0");
        let dist_info = prepared.path().join(config.dist_info_dir());
        fs::create_dir_all(&dist_info).unwrap();
        fs::write(
            dist_info.join("METADATA"),
            b"Metadata-Version: 2.4\nName: demo\n",
        )
        .unwrap();
        fs::write(dist_info.join("WHEEL"), b"Wheel-Version: 1.0\n").unwrap();

        let reused =
            super::copy_prepared_dist_info(prepared.path(), staged.path(), &config).unwrap();
        assert!(reused);
        let staged_dist = staged.path().join(config.dist_info_dir());
        let metadata = fs::read_to_string(staged_dist.join("METADATA")).unwrap();
        assert!(metadata.contains("Name: demo"));
        assert!(staged_dist.join("WHEEL").is_file());
        assert!(!staged_dist.join("RECORD").exists());
    }

    #[test]
    fn copy_prepared_dist_info_returns_false_when_directory_absent() {
        use tempfile::TempDir;

        let prepared = TempDir::new().unwrap();
        let staged = TempDir::new().unwrap();
        let config = config_with_mojo("0.26.2.0");
        let reused =
            super::copy_prepared_dist_info(prepared.path(), staged.path(), &config).unwrap();
        assert!(!reused);
    }

    #[test]
    fn copy_prepared_dist_info_rejects_mismatched_distribution() {
        use std::fs;
        use tempfile::TempDir;

        let prepared = TempDir::new().unwrap();
        let staged = TempDir::new().unwrap();
        let config = config_with_mojo("0.26.2.0");
        let intruder = prepared.path().join("other-0.1.0.dist-info");
        fs::create_dir_all(&intruder).unwrap();
        fs::write(
            intruder.join("METADATA"),
            b"Metadata-Version: 2.4\nName: other\n",
        )
        .unwrap();

        let error =
            super::copy_prepared_dist_info(prepared.path(), staged.path(), &config).unwrap_err();
        assert!(matches!(error, super::MohausError::WheelMetadata { .. }));
    }

    #[test]
    fn copy_dir_skips_dev_artifacts_and_inplace_extensions() {
        use std::fs;
        use tempfile::TempDir;

        let source_root = TempDir::new().unwrap();
        let dest_root = TempDir::new().unwrap();

        fs::write(source_root.path().join("pyproject.toml"), b"").unwrap();
        fs::write(source_root.path().join("README.md"), b"").unwrap();
        fs::write(source_root.path().join("LICENSE"), b"").unwrap();
        fs::write(source_root.path().join(".mojo-version"), b"").unwrap();
        fs::write(source_root.path().join(".gitignore"), b"").unwrap();
        fs::write(source_root.path().join(".gitattributes"), b"").unwrap();
        fs::write(source_root.path().join("flake.nix"), b"").unwrap();

        let pkg = source_root.path().join("demo");
        fs::create_dir_all(pkg.join("__pycache__")).unwrap();
        fs::create_dir_all(pkg.join(".pytest_cache")).unwrap();
        fs::write(pkg.join("__init__.py"), b"").unwrap();
        fs::write(pkg.join("py.typed"), b"").unwrap();
        fs::write(pkg.join("__init__.cpython-311.pyc"), b"").unwrap();
        fs::write(pkg.join("_native.cpython-311-darwin.so"), b"").unwrap();
        fs::write(pkg.join(".#emacs.swp"), b"").unwrap();
        fs::write(pkg.join("__pycache__/cached.pyc"), b"").unwrap();
        fs::write(pkg.join(".pytest_cache/v"), b"").unwrap();

        crate::wheel::copy_dir(source_root.path(), dest_root.path()).unwrap();

        let staged_pkg = dest_root.path().join("demo");
        assert!(staged_pkg.join("__init__.py").is_file());
        assert!(staged_pkg.join("py.typed").is_file());
        assert!(!dest_root.path().join("pyproject.toml").exists());
        assert!(!dest_root.path().join("README.md").exists());
        assert!(!dest_root.path().join("LICENSE").exists());
        assert!(!dest_root.path().join(".mojo-version").exists());
        assert!(!dest_root.path().join(".gitignore").exists());
        assert!(!dest_root.path().join(".gitattributes").exists());
        assert!(!dest_root.path().join("flake.nix").exists());
        assert!(!staged_pkg.join("__pycache__").exists());
        assert!(!staged_pkg.join(".pytest_cache").exists());
        assert!(!staged_pkg.join("__init__.cpython-311.pyc").exists());
        assert!(!staged_pkg.join("_native.cpython-311-darwin.so").exists());
        assert!(!staged_pkg.join(".#emacs.swp").exists());
    }
}
