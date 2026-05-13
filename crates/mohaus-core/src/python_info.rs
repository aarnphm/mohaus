use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{MohausError, Result};
use crate::toolchain::find_program_in_path;

/// Python ABI and platform information needed for extension and wheel names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PythonInfo {
    pub ext_suffix: String,
    pub wheel_tag: String,
    pub pure_tag: String,
    pub mojo_executables: Vec<PathBuf>,
    pub mojo_search_paths: Vec<PathBuf>,
}

impl PythonInfo {
    /// Build Python info from already queried Python values.
    ///
    /// # Errors
    ///
    /// Returns an error when the interpreter is older than Python 3.11.
    pub fn from_parts(
        ext_suffix: String,
        cache_tag: String,
        platform: String,
        major: u8,
        minor: u8,
    ) -> Result<Self> {
        Self::from_parts_with_mojo_paths(
            ext_suffix,
            cache_tag,
            platform,
            major,
            minor,
            Vec::new(),
            Vec::new(),
        )
    }

    /// Build Python info from queried Python values plus Mojo paths discovered
    /// in the active Python environment.
    ///
    /// # Errors
    ///
    /// Returns an error when the interpreter is older than Python 3.11.
    pub fn from_parts_with_mojo_paths(
        ext_suffix: String,
        cache_tag: String,
        platform: String,
        major: u8,
        minor: u8,
        mojo_executables: Vec<PathBuf>,
        mojo_search_paths: Vec<PathBuf>,
    ) -> Result<Self> {
        if major < 3 || (major == 3 && minor < 11) {
            return Err(MohausError::InvalidProject {
                message: format!(
                    "mohaus v1 requires Python >=3.11, but the active interpreter is {major}.{minor}"
                ),
            });
        }
        let platform_tag = platform.replace(['-', '.'], "_");
        let python_tag = python_wheel_tag(&cache_tag, major, minor);
        let abi_tag = abi_wheel_tag(&ext_suffix, &cache_tag, &python_tag);
        Ok(Self {
            ext_suffix,
            wheel_tag: format!("{python_tag}-{abi_tag}-{platform_tag}"),
            pure_tag: "py3-none-any".to_string(),
            mojo_executables: normalize_existing_paths(mojo_executables, Path::is_file),
            mojo_search_paths: normalize_existing_paths(mojo_search_paths, Path::is_dir),
        })
    }

    /// Detect Python info by invoking the active Python interpreter.
    ///
    /// # Errors
    ///
    /// Returns an error when no Python executable is found, introspection fails,
    /// or the interpreter is too old.
    pub fn detect() -> Result<Self> {
        let python = find_program_in_path("python")
            .or_else(|| find_program_in_path("python3"))
            .ok_or_else(|| MohausError::InvalidProject {
                message: "could not find python or python3 on PATH".to_string(),
            })?;
        detect_with_python(&python)
    }
}

fn detect_with_python(python: &PathBuf) -> Result<PythonInfo> {
    let script = r#"
import os
import pathlib
import sys
import sysconfig

def _candidate_roots():
    roots = []
    paths = sysconfig.get_paths()
    for key in ("purelib", "platlib"):
        value = paths.get(key)
        if value:
            roots.append(value)
    roots.extend(entry for entry in sys.path if entry)
    seen = set()
    out = []
    for root in roots:
        path = str(pathlib.Path(root))
        if path not in seen:
            seen.add(path)
            out.append(path)
    return out

def _modular_paths():
    executables = []
    search_paths = []
    scripts = sysconfig.get_path("scripts")
    if scripts:
        executable = pathlib.Path(scripts) / "mojo"
        if executable.is_file():
            executables.append(str(executable))
    for root in _candidate_roots():
        modular = pathlib.Path(root) / "modular"
        executable = modular / "bin" / "mojo"
        search = modular / "lib" / "mojo"
        if executable.is_file():
            executables.append(str(executable))
        if search.is_dir():
            search_paths.append(str(search))
    return executables, search_paths

executables, search_paths = _modular_paths()
print(sysconfig.get_config_var("EXT_SUFFIX") or ".so")
print(sys.implementation.cache_tag or f"py{sys.version_info.major}{sys.version_info.minor}")
print(sysconfig.get_platform())
print(sys.version_info.major)
print(sys.version_info.minor)
print(os.pathsep.join(executables))
print(os.pathsep.join(search_paths))
"#;
    let output = Command::new(python)
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|source| MohausError::CommandIo {
            program: python.display().to_string(),
            source,
        })?;
    if !output.status.success() {
        return Err(MohausError::CommandFailed {
            program: python.display().to_string(),
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    if lines.len() != 7 {
        return Err(MohausError::InvalidProject {
            message: "python introspection returned an unexpected shape".to_string(),
        });
    }
    let major = parse_u8(lines[3], "major Python version")?;
    let minor = parse_u8(lines[4], "minor Python version")?;
    PythonInfo::from_parts_with_mojo_paths(
        lines[0].to_string(),
        lines[1].to_string(),
        lines[2].to_string(),
        major,
        minor,
        split_paths_line(lines[5]),
        split_paths_line(lines[6]),
    )
}

/// Discover Modular's package-managed Mojo compiler and package search roots
/// under Python environment roots such as `site-packages`.
pub fn discover_mojo_paths_from_python_roots<I>(roots: I) -> (Vec<PathBuf>, Vec<PathBuf>)
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut executables = Vec::new();
    let mut search_paths = Vec::new();
    for root in roots {
        let modular = root.join("modular");
        let executable = modular.join("bin").join("mojo");
        let search = modular.join("lib").join("mojo");
        if executable.is_file() {
            executables.push(executable);
        }
        if search.is_dir() {
            search_paths.push(search);
        }
    }
    (
        normalize_existing_paths(executables, Path::is_file),
        normalize_existing_paths(search_paths, Path::is_dir),
    )
}

/// Discover the Python console-script Mojo wrapper from a Python scripts dir.
pub fn discover_mojo_executable_from_python_scripts(scripts: Option<PathBuf>) -> Vec<PathBuf> {
    let Some(scripts) = scripts else {
        return Vec::new();
    };
    normalize_existing_paths(vec![scripts.join("mojo")], Path::is_file)
}

fn split_paths_line(value: &str) -> Vec<PathBuf> {
    if value.is_empty() {
        Vec::new()
    } else {
        env::split_paths(value).collect()
    }
}

fn normalize_existing_paths(paths: Vec<PathBuf>, exists: fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for path in paths {
        if !exists(&path) {
            continue;
        }
        let normalized = fs::canonicalize(&path).unwrap_or(path);
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    out
}

fn parse_u8(value: &str, label: &str) -> Result<u8> {
    value
        .parse::<u8>()
        .map_err(|source| MohausError::InvalidProject {
            message: format!("could not parse {label} `{value}`: {source}"),
        })
}

fn python_wheel_tag(cache_tag: &str, major: u8, minor: u8) -> String {
    if cache_tag.starts_with("cpython-") {
        return format!("cp{major}{minor}");
    }
    cache_tag.replace(['-', '.'], "_")
}

fn abi_wheel_tag(ext_suffix: &str, cache_tag: &str, python_tag: &str) -> String {
    if ext_suffix.contains(".abi3") {
        return "abi3".to_string();
    }
    if cache_tag.starts_with("cpython-") {
        return python_tag.to_string();
    }
    "none".to_string()
}

#[cfg(test)]
mod tests {
    use crate::{MohausError, Result};

    use super::PythonInfo;

    #[test]
    fn cpython_cache_tag_becomes_pep425_wheel_tag() -> Result<()> {
        let info = PythonInfo::from_parts(
            ".cpython-311-darwin.so".to_string(),
            "cpython-311".to_string(),
            "macosx-14.0-arm64".to_string(),
            3,
            11,
        )?;

        assert_eq!(info.wheel_tag, "cp311-cp311-macosx_14_0_arm64");
        assert!(info.mojo_executables.is_empty());
        assert!(info.mojo_search_paths.is_empty());
        Ok(())
    }

    #[test]
    fn abi3_extension_suffix_becomes_abi3_wheel_tag() -> Result<()> {
        let info = PythonInfo::from_parts(
            ".abi3.so".to_string(),
            "cpython-311".to_string(),
            "macosx-14.0-arm64".to_string(),
            3,
            11,
        )?;

        assert_eq!(info.wheel_tag, "cp311-abi3-macosx_14_0_arm64");
        Ok(())
    }

    #[test]
    fn discovers_modular_mojo_paths_from_python_roots() -> Result<()> {
        let temp = tempfile::TempDir::new().map_err(|source| MohausError::InvalidProject {
            message: format!("could not create temporary directory: {source}"),
        })?;
        let scripts = temp.path().join("bin");
        let root = temp.path().join("site-packages");
        let script_executable = scripts.join("mojo");
        let executable = root.join("modular/bin/mojo");
        let search = root.join("modular/lib/mojo");
        std::fs::create_dir_all(&scripts).map_err(|source| MohausError::CreateDir {
            path: scripts.clone(),
            source,
        })?;
        std::fs::create_dir_all(root.join("modular/bin")).map_err(|source| {
            MohausError::CreateDir {
                path: root.join("modular/bin"),
                source,
            }
        })?;
        std::fs::create_dir_all(&search).map_err(|source| MohausError::CreateDir {
            path: search.clone(),
            source,
        })?;
        std::fs::write(&script_executable, "").map_err(|source| MohausError::WriteFile {
            path: script_executable.clone(),
            source,
        })?;
        std::fs::write(&executable, "").map_err(|source| MohausError::WriteFile {
            path: executable.clone(),
            source,
        })?;

        let (executables, search_paths) =
            super::discover_mojo_paths_from_python_roots([root.clone(), root]);
        let script_executables = super::discover_mojo_executable_from_python_scripts(Some(scripts));

        assert_eq!(
            executables,
            vec![std::fs::canonicalize(&executable).map_err(|source| {
                MohausError::ReadFile {
                    path: executable,
                    source,
                }
            })?]
        );
        assert_eq!(
            script_executables,
            vec![std::fs::canonicalize(&script_executable).map_err(|source| {
                MohausError::ReadFile {
                    path: script_executable,
                    source,
                }
            })?]
        );
        assert_eq!(
            search_paths,
            vec![
                std::fs::canonicalize(&search).map_err(|source| MohausError::ReadFile {
                    path: search,
                    source,
                })?
            ]
        );
        Ok(())
    }
}
