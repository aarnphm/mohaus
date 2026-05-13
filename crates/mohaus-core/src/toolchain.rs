use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{MojoVersion, normalize_mojo_version_token};
use crate::error::{MohausError, Result};
use crate::log::{Verbosity, debug};
use crate::python_info::PythonInfo;

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
    resolve_verified_mojo_with_verbosity(expected, Verbosity::default())
}

/// Resolve and validate the Mojo toolchain for a project pin with diagnostics.
///
/// # Errors
///
/// Returns an error when Mojo cannot be found, `mojo --version` fails, or the
/// discovered version does not match the project pin.
pub fn resolve_verified_mojo_with_verbosity(
    expected: &MojoVersion,
    verbosity: Verbosity,
) -> Result<MojoToolchain> {
    let expected = expected.normalized();
    debug(verbosity, 1, || {
        format!("resolving Mojo executable for version {expected}")
    });
    if let Some(executable) = mojo_override_candidate() {
        debug(verbosity, 1, || {
            format!("using $MOHAUS_MOJO candidate {}", executable.display())
        });
        return verify_mojo_candidate_with_verbosity(executable, &expected, verbosity);
    }
    let candidates = path_and_modular_home_candidates();
    debug(verbosity, 2, || {
        format!("found Mojo candidates: {}", format_paths(&candidates))
    });
    resolve_verified_mojo_from_candidates_with_verbosity(expected, candidates, verbosity)
}

/// Resolve the Mojo toolchain, checking the version only when a project pin is
/// configured.
///
/// # Errors
///
/// Returns an error when Mojo cannot be found, `mojo --version` fails, or the
/// discovered version does not match the configured project pin.
pub fn resolve_project_mojo_with_verbosity(
    expected: Option<&MojoVersion>,
    verbosity: Verbosity,
) -> Result<MojoToolchain> {
    resolve_project_mojo_for_python_with_verbosity(expected, None, verbosity)
}

/// Resolve the Mojo toolchain, preferring the compiler shipped in the active
/// Python environment before falling back to PATH and MODULAR_HOME.
///
/// # Errors
///
/// Returns an error when Mojo cannot be found, `mojo --version` fails, or the
/// discovered version does not match the configured project pin.
pub fn resolve_project_mojo_for_python_with_verbosity(
    expected: Option<&MojoVersion>,
    python: Option<&PythonInfo>,
    verbosity: Verbosity,
) -> Result<MojoToolchain> {
    match expected {
        Some(expected) => {
            resolve_verified_mojo_for_python_with_verbosity(expected, python, verbosity)
        }
        None => resolve_unpinned_mojo_for_python_with_verbosity(python, verbosity),
    }
}

fn resolve_unpinned_mojo_for_python_with_verbosity(
    python: Option<&PythonInfo>,
    verbosity: Verbosity,
) -> Result<MojoToolchain> {
    let executable = resolve_mojo_executable_for_python_with_verbosity(python, verbosity)?;
    let version_output = probe_mojo_version_with_verbosity(&executable, verbosity)?;
    debug(verbosity, 1, || {
        format!(
            "using unpinned Mojo {} at {}",
            normalize_mojo_version_token(&version_output),
            executable.display()
        )
    });
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
    resolve_mojo_executable_with_verbosity(Verbosity::default())
}

/// Resolve the Mojo executable without checking the version, with diagnostics.
///
/// # Errors
///
/// Returns an error when no executable is found through `$MOHAUS_MOJO`, `$PATH`,
/// or `$MODULAR_HOME/bin/mojo`.
pub fn resolve_mojo_executable_with_verbosity(verbosity: Verbosity) -> Result<PathBuf> {
    resolve_mojo_executable_for_python_with_verbosity(None, verbosity)
}

fn resolve_mojo_executable_for_python_with_verbosity(
    python: Option<&PythonInfo>,
    verbosity: Verbosity,
) -> Result<PathBuf> {
    debug(verbosity, 1, || "resolving Mojo executable".to_string());
    mojo_candidates_for_python(python)
        .into_iter()
        .inspect(|candidate| {
            debug(verbosity, 1, || {
                format!("using Mojo executable {}", candidate.display())
            });
        })
        .next()
        .ok_or(MohausError::MissingMojo)
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

fn mojo_override_candidate() -> Option<PathBuf> {
    let path = PathBuf::from(env::var_os("MOHAUS_MOJO")?);
    path.is_file().then(|| normalize_mojo_candidate(path))
}

fn mojo_candidates_for_python(python: Option<&PythonInfo>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(candidate) = mojo_override_candidate() {
        push_unique_candidate(&mut candidates, candidate);
    }
    if let Some(python) = python {
        for candidate in &python.mojo_executables {
            push_unique_candidate(&mut candidates, normalize_mojo_candidate(candidate.clone()));
        }
    }
    for candidate in path_and_modular_home_candidates() {
        push_unique_candidate(&mut candidates, candidate);
    }
    candidates
}

fn resolve_verified_mojo_for_python_with_verbosity(
    expected: &MojoVersion,
    python: Option<&PythonInfo>,
    verbosity: Verbosity,
) -> Result<MojoToolchain> {
    let expected = expected.normalized();
    debug(verbosity, 1, || {
        format!("resolving Mojo executable for version {expected}")
    });
    if let Some(executable) = mojo_override_candidate() {
        debug(verbosity, 1, || {
            format!("using $MOHAUS_MOJO candidate {}", executable.display())
        });
        return verify_mojo_candidate_with_verbosity(executable, &expected, verbosity);
    }
    let candidates = mojo_candidates_for_python(python);
    debug(verbosity, 2, || {
        format!("found Mojo candidates: {}", format_paths(&candidates))
    });
    resolve_verified_mojo_from_candidates_with_verbosity(expected, candidates, verbosity)
}

fn path_and_modular_home_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("PATH") {
        for dir in env::split_paths(&path) {
            let candidate = dir.join("mojo");
            if candidate.is_file() {
                push_unique_candidate(&mut candidates, normalize_mojo_candidate(candidate));
            }
        }
    }

    if let Some(modular_home) = env::var_os("MODULAR_HOME") {
        let candidate = PathBuf::from(modular_home).join("bin").join("mojo");
        if candidate.is_file() {
            push_unique_candidate(&mut candidates, normalize_mojo_candidate(candidate));
        }
    }
    candidates
}

fn normalize_mojo_candidate(candidate: PathBuf) -> PathBuf {
    match fs::canonicalize(&candidate) {
        Ok(path) => path,
        Err(_) => candidate,
    }
}

fn push_unique_candidate(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidates.contains(&candidate) {
        candidates.push(candidate);
    }
}

#[cfg(test)]
fn resolve_verified_mojo_from_candidates<I>(
    expected: String,
    candidates: I,
) -> Result<MojoToolchain>
where
    I: IntoIterator<Item = PathBuf>,
{
    resolve_verified_mojo_from_candidates_with_verbosity(expected, candidates, Verbosity::default())
}

fn resolve_verified_mojo_from_candidates_with_verbosity<I>(
    expected: String,
    candidates: I,
    verbosity: Verbosity,
) -> Result<MojoToolchain>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut first_mismatch = None;
    for executable in candidates {
        match verify_mojo_candidate_with_verbosity(executable, &expected, verbosity) {
            Ok(toolchain) => return Ok(toolchain),
            Err(MohausError::MojoVersionMismatch {
                actual, executable, ..
            }) => {
                debug(verbosity, 2, || {
                    format!(
                        "skipping Mojo candidate {} because it reported {}",
                        executable.display(),
                        actual
                    )
                });
                if first_mismatch.is_none() {
                    first_mismatch = Some((executable, actual));
                }
            }
            Err(error) => return Err(error),
        }
    }
    if let Some((executable, actual)) = first_mismatch {
        return Err(MohausError::MojoVersionMismatch {
            expected,
            actual,
            executable,
        });
    }
    Err(MohausError::MissingMojo)
}

fn verify_mojo_candidate_with_verbosity(
    executable: PathBuf,
    expected: &str,
    verbosity: Verbosity,
) -> Result<MojoToolchain> {
    let version_output = probe_mojo_version_with_verbosity(&executable, verbosity)?;
    let actual = normalize_mojo_version_token(&version_output);
    if actual != expected {
        return Err(MohausError::MojoVersionMismatch {
            expected: expected.to_string(),
            actual,
            executable,
        });
    }
    debug(verbosity, 1, || {
        format!("verified Mojo {} at {}", actual, executable.display())
    });
    Ok(MojoToolchain {
        executable,
        version_output,
    })
}

fn probe_mojo_version_with_verbosity(executable: &Path, verbosity: Verbosity) -> Result<String> {
    debug(verbosity, 2, || {
        format!(
            "running {}",
            format_command(executable, &[OsString::from("--version")])
        )
    });
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
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    debug(verbosity, 2, || {
        format!("{} reported {}", executable.display(), version)
    });
    Ok(version)
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
    run_command_with_env_remove_with_verbosity(program, args, env_remove, Verbosity::default())
}

/// Run a command with selected environment variables removed and diagnostics.
///
/// # Errors
///
/// Returns an error when the process cannot be spawned or exits unsuccessfully.
pub fn run_command_with_env_remove_with_verbosity(
    program: &Path,
    args: &[OsString],
    env_remove: &[&str],
    verbosity: Verbosity,
) -> Result<()> {
    let mut command = Command::new(program);
    command.args(args);
    for name in env_remove {
        command.env_remove(name);
    }
    debug(verbosity, 1, || {
        format!("running {}", format_command(program, args))
    });
    if !env_remove.is_empty() {
        debug(verbosity, 2, || {
            format!(
                "removing child environment variables: {}",
                env_remove.join(", ")
            )
        });
    }
    let output = command.output().map_err(|source| MohausError::CommandIo {
        program: program.display().to_string(),
        source,
    })?;
    if output.status.success() {
        debug(verbosity, 2, || {
            format!("{} exited with {}", program.display(), output.status)
        });
        forward_command_output(&output.stdout, &output.stderr);
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

fn forward_command_output(stdout: &[u8], stderr: &[u8]) {
    if !stdout.is_empty() {
        let mut handle = io::stdout().lock();
        let _ = handle.write_all(stdout);
        let _ = handle.flush();
    }
    if !stderr.is_empty() {
        let mut handle = io::stderr().lock();
        let _ = handle.write_all(stderr);
        let _ = handle.flush();
    }
}

fn format_paths(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return "<none>".to_string();
    }
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_command(program: &Path, args: &[OsString]) -> String {
    std::iter::once(format_arg(program.as_os_str()))
        .chain(args.iter().map(|arg| format_arg(arg.as_os_str())))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_arg(arg: &OsStr) -> String {
    let value = arg.to_string_lossy();
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | '=' | ':'))
    {
        value.to_string()
    } else {
        format!("{value:?}")
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::fs;
    use std::path::Path;

    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};
    use tempfile::TempDir;

    use crate::config::{MojoVersion, normalize_mojo_version_token};
    use crate::error::MohausError;
    use crate::python_info::PythonInfo;

    #[test]
    fn normalizes_cli_version() {
        assert_eq!(normalize_mojo_version_token("Mojo 26.2.0 (abcd)"), "26.2.0");
        assert_eq!(normalize_mojo_version_token("0.26.2.0"), "26.2.0");
        assert_eq!(
            normalize_mojo_version_token("Mojo 1.0.0b2.dev2026050805 (dc0cf636)"),
            "1.0.0b2.dev2026050805"
        );
    }

    #[cfg(unix)]
    #[test]
    fn verified_resolver_skips_mismatched_candidate_until_match() {
        let root = TempDir::new().unwrap();
        let stable = root.path().join("stable").join("mojo");
        let dev = root.path().join("dev").join("mojo");
        write_fake_mojo(&stable, "Mojo 0.26.2.0 (d627decc)");
        write_fake_mojo(&dev, "Mojo 1.0.0.dev0 (deadbeef)");
        let expected = MojoVersion::parse("1.0.0.dev0").unwrap();

        let toolchain = super::resolve_verified_mojo_from_candidates(
            expected.normalized(),
            vec![stable, dev.clone()],
        )
        .unwrap();

        assert_eq!(toolchain.executable, dev);
        assert_eq!(toolchain.version_output, "Mojo 1.0.0.dev0 (deadbeef)");
    }

    #[cfg(unix)]
    #[test]
    fn verified_resolver_reports_first_mismatch_when_none_match() {
        let root = TempDir::new().unwrap();
        let stable = root.path().join("stable").join("mojo");
        let nightly = root.path().join("nightly").join("mojo");
        write_fake_mojo(&stable, "Mojo 0.26.2.0 (d627decc)");
        write_fake_mojo(&nightly, "Mojo 1.0.0b2.dev2026050805 (abc123)");
        let expected = MojoVersion::parse("1.0.0.dev0").unwrap();

        let error = super::resolve_verified_mojo_from_candidates(
            expected.normalized(),
            vec![stable.clone(), nightly],
        )
        .unwrap_err();

        match error {
            MohausError::MojoVersionMismatch {
                expected,
                actual,
                executable,
            } => {
                assert_eq!(expected, "1.0.0.dev0");
                assert_eq!(actual, "26.2.0");
                assert_eq!(executable, stable);
            }
            other => panic!("expected version mismatch, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn python_environment_mojo_candidate_precedes_path_candidates() {
        if std::env::var_os("MOHAUS_MOJO").is_some() {
            return;
        }
        let root = TempDir::new().unwrap();
        let python_mojo = root.path().join("site-packages/modular/bin/mojo");
        let path_mojo = root.path().join("bin/mojo");
        write_fake_mojo(&python_mojo, "Mojo 1.0.0b1 (abc123)");
        write_fake_mojo(&path_mojo, "Mojo 1.0.0b1 (abc123)");
        let python = PythonInfo::from_parts_with_mojo_paths(
            ".cpython-311-darwin.so".to_string(),
            "cpython-311".to_string(),
            "macosx-14.0-arm64".to_string(),
            3,
            11,
            vec![python_mojo.clone()],
            Vec::new(),
        )
        .unwrap();

        let candidates = super::mojo_candidates_for_python(Some(&python));

        assert_eq!(
            candidates.first(),
            Some(&fs::canonicalize(python_mojo).unwrap())
        );
    }

    #[cfg(unix)]
    #[test]
    fn normalizes_symlinked_candidate_to_real_executable() {
        let root = TempDir::new().unwrap();
        let target = root
            .path()
            .join("bazel-bin")
            .join("KGEN")
            .join("tools")
            .join("mojo")
            .join("mojo");
        let link = root
            .path()
            .join(".derived")
            .join("build")
            .join("bin")
            .join("mojo");
        write_fake_mojo(&target, "Mojo 1.0.0.dev0 (deadbeef)");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(&target, &link).unwrap();

        assert_eq!(
            super::normalize_mojo_candidate(link),
            fs::canonicalize(target).unwrap()
        );
    }

    #[cfg(unix)]
    fn write_fake_mojo(path: &Path, version: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(
                "#!/bin/sh\n\
                 cat <<'MOHAUS_FAKE_MOJO_VERSION'\n\
                 {version}\n\
                 MOHAUS_FAKE_MOJO_VERSION\n"
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}
