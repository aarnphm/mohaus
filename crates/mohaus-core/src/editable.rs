use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::config::{MojoModule, ProjectConfig};
use crate::error::{MohausError, Result};
use crate::python_info::PythonInfo;
use crate::toolchain::{resolve_verified_mojo, run_command_with_env_remove};
use crate::wheel::write_file;

#[cfg(target_os = "macos")]
const MOJO_RUNTIME_LIBS: &[&str] = &[
    "libKGENCompilerRTShared.dylib",
    "libAsyncRTMojoBindings.dylib",
    "libAsyncRTRuntimeGlobals.dylib",
    "libMSupportGlobals.dylib",
];
#[cfg(target_os = "macos")]
const MOJO_COMPILER_RT: &str = "libKGENCompilerRTShared.dylib";
#[cfg(target_os = "macos")]
const MOJO_LOADER_RPATH: &str = "@loader_path";

#[cfg(target_os = "linux")]
const MOJO_RUNTIME_LIBS: &[&str] = &[
    "libKGENCompilerRTShared.so",
    "libAsyncRTMojoBindings.so",
    "libAsyncRTRuntimeGlobals.so",
    "libMSupportGlobals.so",
];
#[cfg(target_os = "linux")]
const MOJO_COMPILER_RT: &str = "libKGENCompilerRTShared.so";
#[cfg(target_os = "linux")]
const MOJO_LOADER_RPATH: &str = "$ORIGIN";

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
const MOJO_RUNTIME_LIBS: &[&str] = &[];
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
const MOJO_COMPILER_RT: &str = "libKGENCompilerRTShared.so";
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
const MOJO_LOADER_RPATH: &str = "$ORIGIN";

const PYTHON_PACKAGE_MOJO_ENV_REMOVE: &[&str] =
    &["MODULAR_PATH", "MODULAR_DERIVED_PATH", "MODULAR_HOME"];

/// Discovered mojo runtime support libraries that must be co-located with the
/// compiled extension.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MojoRuntimeLibs {
    pub lib_dir: PathBuf,
    pub libs: Vec<PathBuf>,
}

/// Ensure editable in-place extensions are built for a project.
///
/// Skips Mojo toolchain resolution when every module's source hash already
/// matches its sidecar and the in-place extension is on disk. A file lock at
/// `.mohaus/.rebuild-lock` serializes concurrent rebuilds across processes.
///
/// # Errors
///
/// Returns an error when configuration loading, Mojo resolution, source hashing,
/// compilation, or sidecar writing fails.
pub fn ensure_editable_built(project_dir: impl AsRef<Path>, python: &PythonInfo) -> Result<()> {
    let config = ProjectConfig::load(project_dir.as_ref())?;
    let plan = plan_rebuilds(&config, python)?;
    if plan.is_empty() {
        return Ok(());
    }
    let _guard = acquire_rebuild_lock(&config)?;
    let plan = plan_rebuilds(&config, python)?;
    if plan.is_empty() {
        return Ok(());
    }
    let pinned = config
        .mojo_version
        .as_ref()
        .ok_or_else(|| MohausError::InvalidProject {
            message: ".mojo-version is required for editable builds with Mojo modules".to_string(),
        })?;
    let mojo = resolve_verified_mojo(pinned)?;
    for stale in plan {
        compile_module(
            &config,
            &stale.module,
            &stale.extension_path,
            &mojo.executable,
        )?;
        write_file(&stale.hash_path, stale.expected_hash.as_bytes())?;
    }
    Ok(())
}

#[derive(Debug)]
struct StaleModule {
    module: MojoModule,
    extension_path: PathBuf,
    hash_path: PathBuf,
    expected_hash: String,
}

fn plan_rebuilds(config: &ProjectConfig, python: &PythonInfo) -> Result<Vec<StaleModule>> {
    let mut stale = Vec::new();
    for module in &config.modules {
        let hash = source_hash(config, module)?;
        let hash_path = editable_hash_path(config, module);
        let extension_path = extension_output_path(config, module, &python.ext_suffix);
        let current_hash = fs::read_to_string(&hash_path).ok();
        if extension_path.is_file() && current_hash.as_deref() == Some(hash.as_str()) {
            continue;
        }
        stale.push(StaleModule {
            module: module.clone(),
            extension_path,
            hash_path,
            expected_hash: hash,
        });
    }
    Ok(stale)
}

struct RebuildLock {
    file: fs::File,
}

impl Drop for RebuildLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn acquire_rebuild_lock(config: &ProjectConfig) -> Result<RebuildLock> {
    let dir = config.project_dir.join(".mohaus");
    fs::create_dir_all(&dir).map_err(|source| MohausError::CreateDir {
        path: dir.clone(),
        source,
    })?;
    let lock_path = dir.join(".rebuild-lock");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|source| MohausError::WriteFile {
            path: lock_path.clone(),
            source,
        })?;
    FileExt::lock_exclusive(&file).map_err(|source| MohausError::WriteFile {
        path: lock_path,
        source,
    })?;
    Ok(RebuildLock { file })
}

/// Compile a configured Mojo module into a shared library.
///
/// # Errors
///
/// Returns an error when the output directory cannot be created or `mojo build`
/// fails.
pub fn compile_module(
    config: &ProjectConfig,
    module: &MojoModule,
    output: &Path,
    mojo_executable: &Path,
) -> Result<()> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|source| MohausError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let runtime = discover_mojo_runtime_libs(mojo_executable)?;
    let args = build_mojo_args(config, module, output, runtime.as_ref());
    let env_remove = if runtime.is_some() {
        PYTHON_PACKAGE_MOJO_ENV_REMOVE
    } else {
        &[]
    };
    run_command_with_env_remove(mojo_executable, &args, env_remove)?;
    if let Some(runtime) = &runtime {
        copy_mojo_runtime_libs(runtime, output)?;
    }
    Ok(())
}

/// Build the argv passed to `mojo` for a single module compile. Exposed for
/// integration testing so fixtures can assert `-I` is always present.
pub fn build_mojo_args(
    config: &ProjectConfig,
    module: &MojoModule,
    output: &Path,
    runtime: Option<&MojoRuntimeLibs>,
) -> Vec<OsString> {
    let entry = config.project_dir.join(&module.entry);
    let mut args = vec![
        OsString::from("build"),
        entry.into_os_string(),
        OsString::from("--emit"),
        OsString::from("shared-lib"),
        OsString::from("-o"),
        output.as_os_str().to_os_string(),
    ];
    if let Some(runtime) = runtime {
        add_mojo_runtime_link_args(&mut args, runtime);
    }
    for include in &config.mojo_include_paths {
        args.push(OsString::from("-I"));
        args.push(config.project_dir.join(include).into_os_string());
    }
    for flag in &config.mojo_flags {
        args.push(OsString::from(flag));
    }
    args
}

fn add_mojo_runtime_link_args(args: &mut Vec<OsString>, runtime: &MojoRuntimeLibs) {
    args.push(OsString::from("-Xlinker"));
    args.push(runtime.lib_dir.join(MOJO_COMPILER_RT).into_os_string());
    args.push(OsString::from("-Xlinker"));
    args.push(OsString::from("-rpath"));
    args.push(OsString::from("-Xlinker"));
    args.push(OsString::from(MOJO_LOADER_RPATH));
}

fn copy_mojo_runtime_libs(runtime: &MojoRuntimeLibs, output: &Path) -> Result<()> {
    let Some(output_dir) = output.parent() else {
        return Err(MohausError::InvalidProject {
            message: format!(
                "extension output has no parent directory: {}",
                output.display()
            ),
        });
    };
    for lib in &runtime.libs {
        let Some(file_name) = lib.file_name() else {
            return Err(MohausError::InvalidProject {
                message: format!(
                    "mojo runtime library path has no file name: {}",
                    lib.display()
                ),
            });
        };
        let destination = output_dir.join(file_name);
        fs::copy(lib, &destination).map_err(|source| MohausError::CopyFile {
            source_path: lib.clone(),
            dest_path: destination,
            source,
        })?;
    }
    Ok(())
}

fn discover_mojo_runtime_libs(mojo_executable: &Path) -> Result<Option<MojoRuntimeLibs>> {
    let Some(bin_dir) = mojo_executable.parent() else {
        return Ok(None);
    };
    let Some(root) = bin_dir.parent() else {
        return Ok(None);
    };
    let lib_root = root.join("lib");
    if !lib_root.is_dir() {
        return Ok(None);
    }

    for entry in WalkDir::new(&lib_root).max_depth(5) {
        let entry = entry.map_err(|source| MohausError::WalkDir {
            path: lib_root.clone(),
            source,
        })?;
        if entry.file_type().is_dir()
            && entry.path().ends_with("modular/lib")
            && let Some(runtime) = runtime_libs_from_dir(entry.path())?
        {
            return Ok(Some(runtime));
        }
    }
    Ok(None)
}

fn runtime_libs_from_dir(lib_dir: &Path) -> Result<Option<MojoRuntimeLibs>> {
    if MOJO_RUNTIME_LIBS.is_empty() {
        return Ok(None);
    }
    let compiler_rt = lib_dir.join(MOJO_COMPILER_RT);
    if !compiler_rt.is_file() {
        return Ok(None);
    }

    let mut libs = Vec::new();
    let mut missing = Vec::new();
    for name in MOJO_RUNTIME_LIBS {
        let path = lib_dir.join(name);
        if path.is_file() {
            libs.push(path);
        } else {
            missing.push(*name);
        }
    }

    if !missing.is_empty() {
        return Err(MohausError::InvalidProject {
            message: format!(
                "mojo runtime library directory {} is missing {}",
                lib_dir.display(),
                missing.join(", ")
            ),
        });
    }

    Ok(Some(MojoRuntimeLibs {
        lib_dir: lib_dir.to_path_buf(),
        libs,
    }))
}

/// Return the in-place extension path for a module.
pub fn extension_output_path(
    config: &ProjectConfig,
    module: &MojoModule,
    ext_suffix: &str,
) -> PathBuf {
    let mut relative = module.name.relative_path_without_suffix();
    relative.set_file_name(format!("{}{}", module.name.leaf(), ext_suffix));
    config.python_source_root().join(relative)
}

/// Hash a single Mojo source tree without any module / flag / include-path
/// metadata. Exists so the Mojo parity port (`src/mohaus_hashing/`) and this
/// crate exercise the same algorithm against the same fixture corpus.
///
/// # Errors
///
/// Returns an error when the tree cannot be walked or read.
pub fn tree_hash(root: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    hash_tree(root, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Hash all Mojo inputs relevant to an editable rebuild.
///
/// # Errors
///
/// Returns an error when source or include-path files cannot be walked or read.
pub fn source_hash(config: &ProjectConfig, module: &MojoModule) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(module.name.as_str().as_bytes());
    hasher.update(module.entry.to_string_lossy().as_bytes());
    for flag in &config.mojo_flags {
        hasher.update(flag.as_bytes());
        hasher.update(b"\0");
    }
    hash_tree(&config.mojo_source_root(), &mut hasher)?;
    for include in &config.mojo_include_paths {
        let path = config.project_dir.join(include);
        if path.is_dir() {
            hash_tree(&path, &mut hasher)?;
        } else if path.is_file() {
            hash_file(&path, &path, &mut hasher)?;
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn editable_hash_path(config: &ProjectConfig, module: &MojoModule) -> PathBuf {
    config
        .project_dir
        .join(".mohaus")
        .join(format!("{}.hash", module.name.as_str().replace('.', "_")))
}

fn hash_tree(root: &Path, hasher: &mut Sha256) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(root) {
        let entry = entry.map_err(|source| MohausError::WalkDir {
            path: root.to_path_buf(),
            source,
        })?;
        if entry.file_type().is_file() {
            let path = entry.path();
            let relevant = path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| matches!(ext, "mojo" | "🔥" | "mojopkg"));
            if relevant {
                files.push(path.to_path_buf());
            }
        }
    }
    files.sort();
    for path in files {
        hash_file(root, &path, hasher)?;
    }
    Ok(())
}

fn hash_file(root: &Path, path: &Path, hasher: &mut Sha256) -> Result<()> {
    let relative = path.strip_prefix(root).unwrap_or(path);
    hasher.update(relative.to_string_lossy().as_bytes());
    hasher.update(b"\0");
    let bytes = fs::read(path).map_err(|source| MohausError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    hasher.update(bytes);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use crate::config::ProjectConfig;
    use crate::editable::{
        MOJO_COMPILER_RT, MOJO_LOADER_RPATH, MOJO_RUNTIME_LIBS, MojoRuntimeLibs,
        add_mojo_runtime_link_args, discover_mojo_runtime_libs, source_hash,
    };

    #[test]
    fn source_hash_changes_with_mojo_source() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::create_dir_all(temp.path().join("python/demo")).unwrap();
        fs::write(temp.path().join(".mojo-version"), "0.26.2.0").unwrap();
        fs::write(
            temp.path().join("pyproject.toml"),
            r#"
[project]
name = "demo"
version = "0.1.0"

[tool.mohaus]
module-name = "demo._native"
"#,
        )
        .unwrap();
        fs::write(temp.path().join("src/lib.mojo"), "def main():\n  pass\n").unwrap();
        let config = ProjectConfig::load(temp.path()).unwrap();
        let before = source_hash(&config, &config.modules[0]).unwrap();
        fs::write(
            temp.path().join("src/lib.mojo"),
            "def main():\n  print(1)\n",
        )
        .unwrap();
        let after = source_hash(&config, &config.modules[0]).unwrap();
        assert_ne!(before, after);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn discovers_python_package_mojo_runtime_libs() {
        let temp = TempDir::new().unwrap();
        let mojo = temp.path().join("bin").join("mojo");
        fs::create_dir_all(mojo.parent().unwrap()).unwrap();
        fs::write(&mojo, "").unwrap();
        let lib_dir = temp
            .path()
            .join("lib")
            .join("python3.11")
            .join("site-packages")
            .join("modular")
            .join("lib");
        fs::create_dir_all(&lib_dir).unwrap();
        for name in MOJO_RUNTIME_LIBS {
            fs::write(lib_dir.join(name), "").unwrap();
        }

        let runtime = discover_mojo_runtime_libs(&mojo).unwrap().unwrap();

        assert_eq!(runtime.lib_dir, lib_dir);
        assert_eq!(runtime.libs.len(), MOJO_RUNTIME_LIBS.len());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn runtime_link_args_link_compiler_rt_and_loader_rpath() {
        let runtime = MojoRuntimeLibs {
            lib_dir: PathBuf::from("/tmp/mojo/lib"),
            libs: vec![PathBuf::from(format!("/tmp/mojo/lib/{MOJO_COMPILER_RT}"))],
        };
        let mut args = Vec::new();

        add_mojo_runtime_link_args(&mut args, &runtime);

        assert_eq!(
            args,
            [
                "-Xlinker",
                &format!("/tmp/mojo/lib/{MOJO_COMPILER_RT}"),
                "-Xlinker",
                "-rpath",
                "-Xlinker",
                MOJO_LOADER_RPATH,
            ]
            .map(OsString::from)
        );
    }
}
