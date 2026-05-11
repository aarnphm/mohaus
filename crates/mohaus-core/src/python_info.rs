use std::path::PathBuf;
use std::process::Command;

use crate::error::{MohausError, Result};
use crate::toolchain::find_program_in_path;

/// Python ABI and platform information needed for extension and wheel names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PythonInfo {
    pub ext_suffix: String,
    pub wheel_tag: String,
    pub pure_tag: String,
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
import sys
import sysconfig
print(sysconfig.get_config_var("EXT_SUFFIX") or ".so")
print(sys.implementation.cache_tag or f"py{sys.version_info.major}{sys.version_info.minor}")
print(sysconfig.get_platform())
print(sys.version_info.major)
print(sys.version_info.minor)
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
    if lines.len() != 5 {
        return Err(MohausError::InvalidProject {
            message: "python introspection returned an unexpected shape".to_string(),
        });
    }
    let major = parse_u8(lines[3], "major Python version")?;
    let minor = parse_u8(lines[4], "minor Python version")?;
    PythonInfo::from_parts(
        lines[0].to_string(),
        lines[1].to_string(),
        lines[2].to_string(),
        major,
        minor,
    )
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
    use crate::Result;

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
}
