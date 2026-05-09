//! Surgical pyproject.toml edits used by `mohaus add`.
//!
//! Two operations:
//!   - `add_build_system_requirement` appends to `[build-system] requires`.
//!   - `add_mojo_include_path` appends to `[tool.mohaus] mojo-include-paths`.
//!   - `resolve_mojo_dependency` maps `mohaus add --mojo` specs to local
//!     include paths, cloning metadata for git-backed specs when needed.
//!
//! Edits are deterministic: the array becomes a multi-line block sorted by
//! insertion order, with stable indentation. We avoid round-tripping through
//! `toml::Value::serialize` because that re-shapes the entire document.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{MohausError, Result};

const GITHUB_PREFIX: &str = "github:";
const VENDORED_MOJO_DIR: &str = "vendor";

/// Resolved action for a `mohaus add --mojo` spec.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedMojoDependency {
    /// The spec already points at an existing local include root.
    Local { include_path: String },
    /// The spec is a git remote that should be cloned before appending the
    /// checkout path as a local include root.
    Git {
        url: String,
        checkout_dir: PathBuf,
        include_path: String,
    },
}

impl ResolvedMojoDependency {
    /// Local include path to append to `[tool.mohaus].mojo-include-paths`.
    #[must_use]
    pub fn include_path(&self) -> &str {
        match self {
            Self::Local { include_path } | Self::Git { include_path, .. } => include_path,
        }
    }
}

/// Append a PEP 508 requirement to `[build-system].requires`.
///
/// # Errors
///
/// Returns an error when the file cannot be read or written, when no
/// `[build-system]` table exists, or when the `requires` array is malformed.
pub fn add_build_system_requirement(pyproject: &Path, spec: &str) -> Result<()> {
    let original = fs::read_to_string(pyproject).map_err(|source| MohausError::ReadFile {
        path: pyproject.to_path_buf(),
        source,
    })?;
    let updated = append_into_array_in_section(&original, "build-system", "requires", spec)?;
    fs::write(pyproject, updated).map_err(|source| MohausError::WriteFile {
        path: pyproject.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Append a path to `[tool.mohaus].mojo-include-paths`.
///
/// # Errors
///
/// Returns an error when the file cannot be read or written, or when the
/// configured array is malformed.
pub fn add_mojo_include_path(pyproject: &Path, value: &str) -> Result<()> {
    let original = fs::read_to_string(pyproject).map_err(|source| MohausError::ReadFile {
        path: pyproject.to_path_buf(),
        source,
    })?;
    let updated =
        append_into_array_in_section(&original, "tool.mohaus", "mojo-include-paths", value)?;
    fs::write(pyproject, updated).map_err(|source| MohausError::WriteFile {
        path: pyproject.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Resolve a Mojo dependency spec.
///
/// Local paths are normalized relative to the project root. Git specs are
/// converted into `vendor/<repo>` checkout metadata and that checkout path is
/// returned as the include path. Existing local paths win over bare
/// `owner/repo` GitHub shorthand so local vendor paths keep working.
///
/// # Errors
///
/// Returns an error when a local path doesn't exist and the spec is not a
/// supported git form.
pub fn resolve_mojo_dependency(project_dir: &Path, spec: &str) -> Result<ResolvedMojoDependency> {
    if let Some(remote) = parse_explicit_git_spec(spec)? {
        return Ok(resolve_git_dependency(project_dir, &remote));
    }

    if let Some(include_path) = resolve_existing_local_include(project_dir, spec) {
        return Ok(ResolvedMojoDependency::Local { include_path });
    }

    if let Some(remote) = parse_github_slug(spec) {
        return Ok(resolve_git_dependency(project_dir, &remote));
    }

    let candidate = PathBuf::from(spec);
    let absolute = if candidate.is_absolute() {
        candidate.clone()
    } else {
        project_dir.join(&candidate)
    };
    Err(MohausError::InvalidProject {
        message: format!(
            "mojo dependency `{spec}` does not exist (looked for {})",
            absolute.display()
        ),
    })
}

/// Resolve a local Mojo dependency spec.
///
/// # Errors
///
/// Returns an error when the local path doesn't exist.
pub fn resolve_mojo_include(project_dir: &Path, spec: &str) -> Result<String> {
    resolve_existing_local_include(project_dir, spec).ok_or_else(|| {
        let candidate = PathBuf::from(spec);
        let absolute = if candidate.is_absolute() {
            candidate
        } else {
            project_dir.join(candidate)
        };
        MohausError::InvalidProject {
            message: format!(
                "mojo dependency `{spec}` does not exist (looked for {})",
                absolute.display()
            ),
        }
    })
}

fn resolve_existing_local_include(project_dir: &Path, spec: &str) -> Option<String> {
    let candidate = PathBuf::from(spec);
    let absolute = if candidate.is_absolute() {
        candidate.clone()
    } else {
        project_dir.join(&candidate)
    };
    if !absolute.exists() {
        return None;
    }
    let relative = pathdiff_relative(&absolute, project_dir).unwrap_or(absolute.clone());
    Some(unix_path_string(&relative))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GitMojoRemote {
    url: String,
    repo_name: String,
}

fn resolve_git_dependency(project_dir: &Path, remote: &GitMojoRemote) -> ResolvedMojoDependency {
    let checkout_relative = PathBuf::from(VENDORED_MOJO_DIR).join(&remote.repo_name);
    ResolvedMojoDependency::Git {
        url: remote.url.clone(),
        checkout_dir: project_dir.join(&checkout_relative),
        include_path: unix_path_string(&checkout_relative),
    }
}

fn parse_explicit_git_spec(spec: &str) -> Result<Option<GitMojoRemote>> {
    if let Some(value) = spec.strip_prefix(GITHUB_PREFIX) {
        let remote = parse_github_slug(value).ok_or_else(|| MohausError::InvalidProject {
            message: format!("invalid GitHub Mojo dependency `{spec}`; expected github:owner/repo"),
        })?;
        return Ok(Some(remote));
    }

    if is_url_git_spec(spec) || is_scp_git_spec(spec) {
        let repo_name =
            repo_name_from_git_url(spec).ok_or_else(|| MohausError::InvalidProject {
                message: format!(
                    "could not infer repository name from Mojo git dependency `{spec}`"
                ),
            })?;
        return Ok(Some(GitMojoRemote {
            url: spec.to_string(),
            repo_name,
        }));
    }

    Ok(None)
}

fn parse_github_slug(value: &str) -> Option<GitMojoRemote> {
    let (owner, repo) = value.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    if !is_github_component(owner) || !is_github_component(repo) {
        return None;
    }
    let repo_name = strip_git_suffix(repo)?;
    Some(GitMojoRemote {
        url: format!("https://github.com/{owner}/{repo_name}.git"),
        repo_name: repo_name.to_string(),
    })
}

fn is_github_component(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn is_url_git_spec(spec: &str) -> bool {
    ["https://", "ssh://", "git://", "file://"]
        .iter()
        .any(|prefix| spec.starts_with(prefix))
}

fn is_scp_git_spec(spec: &str) -> bool {
    spec.contains('@') && spec.contains(':') && !spec.contains("://")
}

fn repo_name_from_git_url(value: &str) -> Option<String> {
    let without_trailing_slash = value.trim_end_matches('/');
    let path = if is_scp_git_spec(without_trailing_slash) {
        without_trailing_slash.rsplit_once(':')?.1
    } else {
        without_trailing_slash.rsplit('/').next()?
    };
    strip_git_suffix(path).map(str::to_string)
}

fn strip_git_suffix(value: &str) -> Option<&str> {
    let repo_name = value.strip_suffix(".git").unwrap_or(value);
    (!repo_name.is_empty()).then_some(repo_name)
}

fn pathdiff_relative(target: &Path, base: &Path) -> Option<PathBuf> {
    let target = target.canonicalize().ok()?;
    let base = base.canonicalize().ok()?;
    let mut target_iter = target.components();
    let mut base_iter = base.components();
    loop {
        let t = target_iter.clone().next();
        let b = base_iter.clone().next();
        match (t, b) {
            (Some(t), Some(b)) if t == b => {
                target_iter.next();
                base_iter.next();
            }
            _ => break,
        }
    }
    let mut output = PathBuf::new();
    for _ in base_iter {
        output.push("..");
    }
    output.push(target_iter.as_path());
    Some(output)
}

fn unix_path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn append_into_array_in_section(
    document: &str,
    section_path: &str,
    key: &str,
    value: &str,
) -> Result<String> {
    let header = format!("[{section_path}]");
    let mut lines: Vec<String> = document.lines().map(str::to_string).collect();
    let trailing_newline = document.ends_with('\n');
    let section_index = lines
        .iter()
        .position(|line| line.trim() == header)
        .ok_or_else(|| MohausError::WheelMetadata {
            message: format!("could not find `{header}` in pyproject.toml"),
        })?;

    let key_pattern = format!("{key} =");
    let next_section_index = lines
        .iter()
        .enumerate()
        .skip(section_index + 1)
        .find(|(_, line)| line.trim_start().starts_with('[') && line.trim_end().ends_with(']'))
        .map(|(index, _)| index)
        .unwrap_or(lines.len());

    let key_index = lines
        .iter()
        .enumerate()
        .skip(section_index + 1)
        .take(next_section_index - section_index - 1)
        .find(|(_, line)| line.trim_start().starts_with(&key_pattern))
        .map(|(index, _)| index);

    if let Some(start_index) = key_index {
        let end_index = find_array_end(&lines, start_index)?;
        if array_contains_value(&lines[start_index..=end_index], value) {
            return Ok(if trailing_newline {
                format!("{}\n", lines.join("\n"))
            } else {
                lines.join("\n")
            });
        }
        let replacement =
            expand_array_with_appended_value(&lines[start_index..=end_index], key, value)?;
        lines.splice(start_index..=end_index, replacement);
    } else {
        let insert_at = next_section_index;
        let new_line = format!("{key} = [{}]", quoted(value));
        lines.insert(insert_at, new_line);
    }

    let mut joined = lines.join("\n");
    if trailing_newline {
        joined.push('\n');
    }
    Ok(joined)
}

fn find_array_end(lines: &[String], start_index: usize) -> Result<usize> {
    let mut depth = 0_i32;
    let mut found_open = false;
    for (offset, line) in lines.iter().enumerate().skip(start_index) {
        for ch in line.chars() {
            match ch {
                '[' => {
                    depth += 1;
                    found_open = true;
                }
                ']' => {
                    depth -= 1;
                    if found_open && depth == 0 {
                        return Ok(offset);
                    }
                }
                _ => {}
            }
        }
    }
    Err(MohausError::WheelMetadata {
        message: "malformed array literal in pyproject.toml".to_string(),
    })
}

fn array_contains_value(slice: &[String], value: &str) -> bool {
    let needle = quoted(value);
    slice.iter().any(|line| line.contains(&needle))
}

fn expand_array_with_appended_value(
    slice: &[String],
    key: &str,
    value: &str,
) -> Result<Vec<String>> {
    let mut entries = parse_array_entries(slice)?;
    entries.push(value.to_string());
    let mut output = Vec::new();
    output.push(format!("{key} = ["));
    for entry in &entries {
        output.push(format!("  {},", quoted(entry)));
    }
    output.push("]".to_string());
    Ok(output)
}

fn parse_array_entries(slice: &[String]) -> Result<Vec<String>> {
    let joined = slice.join("\n");
    let open = joined.find('[').ok_or_else(|| MohausError::WheelMetadata {
        message: "expected `[` in array literal".to_string(),
    })?;
    let close = joined
        .rfind(']')
        .ok_or_else(|| MohausError::WheelMetadata {
            message: "expected `]` in array literal".to_string(),
        })?;
    if close <= open {
        return Err(MohausError::WheelMetadata {
            message: "malformed array literal".to_string(),
        });
    }
    let body = &joined[open + 1..close];
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escape = false;
    for ch in body.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '"' => {
                in_string = !in_string;
            }
            '\\' if in_string => {
                escape = true;
            }
            ',' if !in_string => {
                push_entry(&mut entries, &current);
                current.clear();
            }
            _ => {
                if in_string {
                    current.push(ch);
                }
            }
        }
    }
    push_entry(&mut entries, &current);
    Ok(entries)
}

fn push_entry(entries: &mut Vec<String>, value: &str) {
    let trimmed = value.trim().trim_matches('"').to_string();
    if !trimmed.is_empty() {
        entries.push(trimmed);
    }
}

fn quoted(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn pyproject_skeleton() -> String {
        r#"[build-system]
requires = ["mohaus>=0.1,<0.2"]
build-backend = "mohaus.backend"

[project]
name = "demo"
version = "0.1.0"

[tool.mohaus]
mojo-src = "src"
python-src = "python"
mojo-include-paths = []
"#
        .to_string()
    }

    #[test]
    fn appends_to_empty_mojo_include_paths() {
        let updated = append_into_array_in_section(
            &pyproject_skeleton(),
            "tool.mohaus",
            "mojo-include-paths",
            "vendor/some_pkg",
        )
        .unwrap();
        assert!(updated.contains("\"vendor/some_pkg\","));
        assert!(updated.contains("[tool.mohaus]"));
    }

    #[test]
    fn appends_to_build_system_requires() {
        let updated = append_into_array_in_section(
            &pyproject_skeleton(),
            "build-system",
            "requires",
            "mojo==0.26.2.0",
        )
        .unwrap();
        assert!(updated.contains("\"mohaus>=0.1,<0.2\","));
        assert!(updated.contains("\"mojo==0.26.2.0\","));
    }

    #[test]
    fn idempotent_when_already_present() {
        let once = append_into_array_in_section(
            &pyproject_skeleton(),
            "tool.mohaus",
            "mojo-include-paths",
            "vendor/some_pkg",
        )
        .unwrap();
        let twice = append_into_array_in_section(
            &once,
            "tool.mohaus",
            "mojo-include-paths",
            "vendor/some_pkg",
        )
        .unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn resolves_existing_local_include_before_bare_github_shorthand() {
        let project = tempfile::tempdir().unwrap();
        let local = project
            .path()
            .join("Mojo-Numerics-and-Algorithms-group")
            .join("NuMojo");
        fs::create_dir_all(&local).unwrap();

        let resolved =
            resolve_mojo_dependency(project.path(), "Mojo-Numerics-and-Algorithms-group/NuMojo")
                .unwrap();

        assert_eq!(
            resolved,
            ResolvedMojoDependency::Local {
                include_path: "Mojo-Numerics-and-Algorithms-group/NuMojo".to_string(),
            }
        );
    }

    #[test]
    fn resolves_github_prefix_to_vendor_checkout() {
        let project = tempfile::tempdir().unwrap();
        let resolved = resolve_mojo_dependency(
            project.path(),
            "github:Mojo-Numerics-and-Algorithms-group/NuMojo",
        )
        .unwrap();

        assert_eq!(
            resolved,
            ResolvedMojoDependency::Git {
                url: "https://github.com/Mojo-Numerics-and-Algorithms-group/NuMojo.git".to_string(),
                checkout_dir: project.path().join("vendor").join("NuMojo"),
                include_path: "vendor/NuMojo".to_string(),
            }
        );
    }

    #[test]
    fn resolves_missing_owner_repo_as_github_checkout() {
        let project = tempfile::tempdir().unwrap();
        let resolved =
            resolve_mojo_dependency(project.path(), "Mojo-Numerics-and-Algorithms-group/NuMojo")
                .unwrap();

        assert_eq!(
            resolved,
            ResolvedMojoDependency::Git {
                url: "https://github.com/Mojo-Numerics-and-Algorithms-group/NuMojo.git".to_string(),
                checkout_dir: project.path().join("vendor").join("NuMojo"),
                include_path: "vendor/NuMojo".to_string(),
            }
        );
    }

    #[test]
    fn rejects_missing_section() {
        let document = "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n";
        let error =
            append_into_array_in_section(document, "tool.mohaus", "mojo-include-paths", "vendor/x")
                .unwrap_err();
        match error {
            MohausError::WheelMetadata { message } => {
                assert!(message.contains("[tool.mohaus]"));
            }
            other => panic!("expected WheelMetadata, got {other:?}"),
        }
    }

    #[test]
    fn quoted_escapes_quotes_and_backslashes() {
        assert_eq!(quoted(r#"a"b\c"#), "\"a\\\"b\\\\c\"");
    }
}
