use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{MojoVersion, normalize_mojo_version_token};
use crate::error::{MohausError, Result};

/// Resolved Mojo executable and version output.
#[derive(Clone, Debug)]
pub struct MojoToolchain {
    pub executable: PathBuf,
    pub version_output: String,
}

/// Resolve and validate the Mojo toolchain for a project pin.
///
/// # Errors
///
/// Returns an error when Mojo cannot be found, `mojo --version` fails, or the
/// discovered version does not match the project pin.
pub fn resolve_verified_mojo(expected: &MojoVersion) -> Result<MojoToolchain> {
    let executable = resolve_mojo_executable()?;
    let version_output = probe_mojo_version(&executable)?;
    let actual = normalize_mojo_version_token(&version_output);
    if actual != expected.normalized() {
        return Err(MohausError::MojoVersionMismatch {
            expected: expected.normalized(),
            actual,
            executable,
        });
    }
    Ok(MojoToolchain {
        executable,
        version_output,
    })
}

/// Resolve the Mojo executable without checking the version.
///
/// # Errors
///
/// Returns an error when no executable is found through `$MOHAUS_MOJO`, `$PATH`,
/// or `$MODULAR_HOME/bin/mojo`.
pub fn resolve_mojo_executable() -> Result<PathBuf> {
    if let Some(value) = env::var_os("MOHAUS_MOJO") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
    }

    if let Some(path) = find_program_in_path("mojo") {
        return Ok(path);
    }

    if let Some(modular_home) = env::var_os("MODULAR_HOME") {
        let candidate = PathBuf::from(modular_home).join("bin").join("mojo");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(MohausError::MissingMojo)
}

/// Find a program in `PATH`.
pub fn find_program_in_path(program: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    find_program_in_paths(program, env::split_paths(&path))
}

fn find_program_in_paths<I>(program: &str, paths: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    for dir in paths {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn probe_mojo_version(executable: &Path) -> Result<String> {
    let output = Command::new(executable)
        .arg("--version")
        .output()
        .map_err(|source| MohausError::CommandIo {
            program: executable.display().to_string(),
            source,
        })?;
    if !output.status.success() {
        return Err(MohausError::CommandFailed {
            program: executable.display().to_string(),
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run a command and return an actionable error on failure.
///
/// # Errors
///
/// Returns an error when the process cannot be spawned or exits unsuccessfully.
pub fn run_command(program: &Path, args: &[OsString]) -> Result<()> {
    run_command_with_env_remove(program, args, &[])
}

/// Run a command with selected environment variables removed.
///
/// # Errors
///
/// Returns an error when the process cannot be spawned or exits unsuccessfully.
pub fn run_command_with_env_remove(
    program: &Path,
    args: &[OsString],
    env_remove: &[&str],
) -> Result<()> {
    let mut command = Command::new(program);
    command.args(args);
    for name in env_remove {
        command.env_remove(name);
    }
    let output = command.output().map_err(|source| MohausError::CommandIo {
        program: program.display().to_string(),
        source,
    })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = if output.stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).to_string()
    } else {
        String::from_utf8_lossy(&output.stderr).to_string()
    };
    Err(MohausError::CommandFailed {
        program: program.display().to_string(),
        status: output.status.to_string(),
        stderr,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use crate::config::normalize_mojo_version_token;

    #[test]
    fn normalizes_cli_version() {
        assert_eq!(normalize_mojo_version_token("Mojo 26.2.0 (abcd)"), "26.2.0");
        assert_eq!(normalize_mojo_version_token("0.26.2.0"), "26.2.0");
        assert_eq!(
            normalize_mojo_version_token("Mojo 1.0.0b2.dev2026050306 (dc0cf636)"),
            "1.0.0b2.dev2026050306"
        );
    }
}
