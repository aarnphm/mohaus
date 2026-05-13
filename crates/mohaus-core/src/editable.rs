use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::config::{MojoModule, ProjectConfig};
use crate::error::{MohausError, Result};
use crate::log::{Verbosity, debug};
use crate::python_info::PythonInfo;
use crate::stub::{ModuleStub, module_stub_plan_for_extension};
use crate::toolchain::{
    resolve_project_mojo_for_python_with_verbosity, run_command_with_env_remove_with_verbosity,
};
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
    ensure_editable_built_with_verbosity(project_dir, python, Verbosity::default())
}

/// Ensure editable in-place extensions are built with human-facing diagnostics.
///
/// # Errors
///
/// Returns an error when configuration loading, Mojo resolution, source hashing,
/// compilation, or sidecar writing fails.
pub fn ensure_editable_built_with_verbosity(
    project_dir: impl AsRef<Path>,
    python: &PythonInfo,
    verbosity: Verbosity,
) -> Result<()> {
    let config = ProjectConfig::load(project_dir.as_ref())?;
    let plan = plan_rebuilds(&config, python)?;
    if plan.is_empty() {
        debug(verbosity, 2, || {
            format!(
                "editable outputs for {} are already fresh",
                config.package.as_str()
            )
        });
        return Ok(());
    }
    debug(verbosity, 1, || {
        format!(
            "editable rebuild needs {} module(s) for {}",
            plan.len(),
            config.package.as_str()
        )
    });
    let _guard = acquire_rebuild_lock(&config)?;
    debug(verbosity, 2, || {
        format!(
            "acquired editable rebuild lock in {}",
            config.project_dir.display()
        )
    });
    let plan = plan_rebuilds(&config, python)?;
    if plan.is_empty() {
        debug(verbosity, 2, || {
            format!(
                "editable outputs for {} became fresh while waiting for the lock",
                config.package.as_str()
            )
        });
        return Ok(());
    }
    let mojo = if plan.iter().any(|stale| stale.needs_compile) {
        Some(resolve_project_mojo_for_python_with_verbosity(
            config.mojo_version.as_ref(),
            Some(python),
            verbosity,
        )?)
    } else {
        None
    };
    for stale in plan {
        if stale.needs_compile {
            let Some(mojo) = mojo.as_ref() else {
                return Err(MohausError::InvalidProject {
                    message:
                        "editable rebuild requested Mojo compilation without a resolved toolchain"
                            .to_string(),
                });
            };
            debug(verbosity, 1, || {
                format!(
                    "rebuilding editable Mojo module {} -> {}",
                    stale.module.name.as_str(),
                    stale.extension_path.display()
                )
            });
            compile_module_with_verbosity(
                &config,
                &stale.module,
                &stale.extension_path,
                &mojo.executable,
                &python.mojo_search_paths,
                verbosity,
            )?;
            write_file(&stale.hash_path, stale.expected_hash.as_bytes())?;
            debug(verbosity, 3, || {
                format!("wrote editable source hash {}", stale.hash_path.display())
            });
        } else if let Some(stub) = stale.stub {
            write_file(&stub.path, stub.text.as_bytes())?;
            debug(verbosity, 2, || {
                format!("refreshed editable stub {}", stub.path.display())
            });
        }
    }
    Ok(())
}

#[derive(Debug)]
struct StaleModule {
    module: MojoModule,
    extension_path: PathBuf,
    hash_path: PathBuf,
    stub: Option<ModuleStub>,
    expected_hash: String,
    needs_compile: bool,
}

fn plan_rebuilds(config: &ProjectConfig, python: &PythonInfo) -> Result<Vec<StaleModule>> {
    let mut stale = Vec::new();
    for module in &config.modules {
        let hash = source_hash_with_mojo_search_paths(config, module, &python.mojo_search_paths)?;
        let hash_path = editable_hash_path(config, module);
        let extension_path = extension_output_path(config, module, &python.ext_suffix);
        let stub = if config.generate_stub {
            Some(module_stub_plan_for_extension(
                config,
                module,
                &extension_path,
            )?)
        } else {
            None
        };
        let current_hash = fs::read_to_string(&hash_path).ok();
        let extension_fresh =
            extension_path.is_file() && current_hash.as_deref() == Some(hash.as_str());
        let stub_fresh = match &stub {
            Some(stub) => stub_matches(&stub.path, &stub.text)?,
            None => true,
        };
        if extension_fresh && stub_fresh {
            continue;
        }
        stale.push(StaleModule {
            module: module.clone(),
            extension_path,
            hash_path,
            stub,
            expected_hash: hash,
            needs_compile: !extension_fresh,
        });
    }
    Ok(stale)
}

fn stub_matches(path: &Path, expected: &str) -> Result<bool> {
    match fs::read_to_string(path) {
        Ok(current) => Ok(current == expected),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(MohausError::ReadFile {
            path: path.to_path_buf(),
            source,
        }),
    }
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
    compile_module_with_verbosity(
        config,
        module,
        output,
        mojo_executable,
        &[],
        Verbosity::default(),
    )
}

/// Compile a configured Mojo module into a shared library with diagnostics.
///
/// # Errors
///
/// Returns an error when the output directory cannot be created or `mojo build`
/// fails.
pub fn compile_module_with_verbosity(
    config: &ProjectConfig,
    module: &MojoModule,
    output: &Path,
    mojo_executable: &Path,
    mojo_search_paths: &[PathBuf],
    verbosity: Verbosity,
) -> Result<()> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|source| MohausError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let runtime = discover_mojo_runtime_libs_with_verbosity(mojo_executable, verbosity)?;
    let args = build_mojo_args_with_search_paths(
        config,
        module,
        output,
        runtime.as_ref(),
        mojo_search_paths,
    );
    let env_remove = if runtime.is_some() {
        PYTHON_PACKAGE_MOJO_ENV_REMOVE
    } else {
        &[]
    };
    run_command_with_env_remove_with_verbosity(mojo_executable, &args, env_remove, verbosity)?;
    if let Some(runtime) = &runtime {
        copy_mojo_runtime_libs(runtime, output, verbosity)?;
    }
    if config.generate_stub {
        let stub_path = crate::stub::write_module_stub_for_extension(config, module, output)?;
        debug(verbosity, 3, || {
            format!("wrote Python stub {}", stub_path.display())
        });
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
    build_mojo_args_with_search_paths(config, module, output, runtime, &[])
}

/// Build the argv passed to `mojo`, including build-environment Mojo package
/// search roots discovered from Python.
pub fn build_mojo_args_with_search_paths(
    config: &ProjectConfig,
    module: &MojoModule,
    output: &Path,
    runtime: Option<&MojoRuntimeLibs>,
    mojo_search_paths: &[PathBuf],
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
    for search_path in mojo_search_paths {
        args.push(OsString::from("-mojo-search-paths"));
        args.push(search_path.as_os_str().to_os_string());
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

fn copy_mojo_runtime_libs(
    runtime: &MojoRuntimeLibs,
    output: &Path,
    verbosity: Verbosity,
) -> Result<()> {
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
        let destination_for_error = destination.clone();
        fs::copy(lib, &destination).map_err(|source| MohausError::CopyFile {
            source_path: lib.clone(),
            dest_path: destination_for_error,
            source,
        })?;
        debug(verbosity, 3, || {
            format!(
                "copied Mojo runtime lib {} -> {}",
                lib.display(),
                destination.display()
            )
        });
    }
    Ok(())
}

#[cfg(test)]
fn discover_mojo_runtime_libs(mojo_executable: &Path) -> Result<Option<MojoRuntimeLibs>> {
    discover_mojo_runtime_libs_with_verbosity(mojo_executable, Verbosity::default())
}

fn discover_mojo_runtime_libs_with_verbosity(
    mojo_executable: &Path,
    verbosity: Verbosity,
) -> Result<Option<MojoRuntimeLibs>> {
    let Some(bin_dir) = mojo_executable.parent() else {
        debug(verbosity, 3, || {
            format!(
                "Mojo executable has no parent: {}",
                mojo_executable.display()
            )
        });
        return Ok(None);
    };
    let Some(root) = bin_dir.parent() else {
        debug(verbosity, 3, || {
            format!(
                "Mojo executable has no toolchain root: {}",
                mojo_executable.display()
            )
        });
        return Ok(None);
    };
    let lib_root = root.join("lib");
    if !lib_root.is_dir() {
        debug(verbosity, 3, || {
            format!("Mojo runtime lib root is absent: {}", lib_root.display())
        });
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
            debug(verbosity, 2, || {
                format!("found Mojo runtime libs in {}", runtime.lib_dir.display())
            });
            return Ok(Some(runtime));
        }
    }
    debug(verbosity, 3, || {
        format!("no Mojo runtime libs found under {}", lib_root.display())
    });
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
    source_hash_with_mojo_search_paths(config, module, &[])
}

fn source_hash_with_mojo_search_paths(
    config: &ProjectConfig,
    module: &MojoModule,
    mojo_search_paths: &[PathBuf],
) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(module.name.as_str().as_bytes());
    hasher.update(module.entry.to_string_lossy().as_bytes());
    if let Some(version) = &config.mojo_version {
        hasher.update(b"mojo-version\0");
        hasher.update(version.normalized().as_bytes());
        hasher.update(b"\0");
    }
    for flag in &config.mojo_flags {
        hasher.update(flag.as_bytes());
        hasher.update(b"\0");
    }
    for path in mojo_search_paths {
        hasher.update(b"mojo-search-path\0");
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        if path.is_dir() {
            hash_tree(path, &mut hasher)?;
        }
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
    use crate::log::Verbosity;
    use crate::python_info::PythonInfo;

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

    #[test]
    fn plan_rebuilds_recompiles_when_mojo_version_changes() {
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
generate-stub = false
"#,
        )
        .unwrap();
        fs::write(temp.path().join("src/lib.mojo"), "def main():\n  pass\n").unwrap();
        let old_config = ProjectConfig::load(temp.path()).unwrap();
        let python = PythonInfo::from_parts(
            ".cpython-311-darwin.so".to_string(),
            "cpython-311".to_string(),
            "macosx-11.0-arm64".to_string(),
            3,
            11,
        )
        .unwrap();
        let module = &old_config.modules[0];
        let extension_path = super::extension_output_path(&old_config, module, &python.ext_suffix);
        fs::write(&extension_path, "").unwrap();
        let hash_path = super::editable_hash_path(&old_config, module);
        fs::create_dir_all(hash_path.parent().unwrap()).unwrap();
        fs::write(&hash_path, source_hash(&old_config, module).unwrap()).unwrap();

        fs::write(temp.path().join(".mojo-version"), "1.0.0.dev0").unwrap();
        let new_config = ProjectConfig::load(temp.path()).unwrap();
        let plan = super::plan_rebuilds(&new_config, &python).unwrap();

        assert_eq!(plan.len(), 1);
        assert!(plan[0].needs_compile);
    }

    #[test]
    fn plan_rebuilds_refreshes_missing_stub_without_recompile() {
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
        fs::write(
            temp.path().join("src/lib.mojo"),
            r#"
from std.python.bindings import PythonModuleBuilder

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_function[passthrough]("passthrough")
    return module.finalize()

def passthrough(value: PythonObject) raises -> PythonObject:
    return value
"#,
        )
        .unwrap();
        let config = ProjectConfig::load(temp.path()).unwrap();
        let python = PythonInfo::from_parts(
            ".cpython-311-darwin.so".to_string(),
            "cpython-311".to_string(),
            "macosx-11.0-arm64".to_string(),
            3,
            11,
        )
        .unwrap();
        let module = &config.modules[0];
        let extension_path = super::extension_output_path(&config, module, &python.ext_suffix);
        fs::write(&extension_path, "").unwrap();
        let hash_path = super::editable_hash_path(&config, module);
        fs::create_dir_all(hash_path.parent().unwrap()).unwrap();
        fs::write(&hash_path, source_hash(&config, module).unwrap()).unwrap();

        let plan = super::plan_rebuilds(&config, &python).unwrap();

        assert_eq!(plan.len(), 1);
        assert!(!plan[0].needs_compile);
        assert_eq!(
            plan[0].stub.as_ref().unwrap().path,
            temp.path().join("python/demo/_native.pyi")
        );
        assert!(
            plan[0]
                .stub
                .as_ref()
                .unwrap()
                .text
                .contains("def passthrough")
        );
    }

    #[test]
    fn plan_rebuilds_ignores_missing_stub_when_generation_disabled() {
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
generate-stub = false
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("src/lib.mojo"),
            r#"
from std.python.bindings import PythonModuleBuilder

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_function[passthrough]("passthrough")
    return module.finalize()

def passthrough(value: PythonObject) raises -> PythonObject:
    return value
"#,
        )
        .unwrap();
        let config = ProjectConfig::load(temp.path()).unwrap();
        let python = PythonInfo::from_parts(
            ".cpython-311-darwin.so".to_string(),
            "cpython-311".to_string(),
            "macosx-11.0-arm64".to_string(),
            3,
            11,
        )
        .unwrap();
        let module = &config.modules[0];
        let extension_path = super::extension_output_path(&config, module, &python.ext_suffix);
        fs::write(&extension_path, "").unwrap();
        let hash_path = super::editable_hash_path(&config, module);
        fs::create_dir_all(hash_path.parent().unwrap()).unwrap();
        fs::write(&hash_path, source_hash(&config, module).unwrap()).unwrap();

        let plan = super::plan_rebuilds(&config, &python).unwrap();

        assert!(plan.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn compile_module_writes_stub_next_to_extension() {
        use std::os::unix::fs::PermissionsExt;

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
        fs::write(
            temp.path().join("src/lib.mojo"),
            r#"
from std.python.bindings import PythonModuleBuilder

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_function[passthrough]("passthrough")
    return module.finalize()

def passthrough(value: PythonObject) raises -> PythonObject:
    return value
"#,
        )
        .unwrap();
        let mojo = temp.path().join("bin/mojo");
        fs::create_dir_all(mojo.parent().unwrap()).unwrap();
        fs::write(&mojo, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&mojo).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&mojo, permissions).unwrap();

        let config = ProjectConfig::load(temp.path()).unwrap();
        let output = temp
            .path()
            .join("python/demo/_native.cpython-311-darwin.so");

        super::compile_module_with_verbosity(
            &config,
            &config.modules[0],
            &output,
            &mojo,
            &[],
            Verbosity::default(),
        )
        .unwrap();

        let stub = fs::read_to_string(temp.path().join("python/demo/_native.pyi")).unwrap();
        assert!(stub.contains("def passthrough(value: object) -> object"));
    }

    #[cfg(unix)]
    #[test]
    fn compile_module_skips_stub_when_generation_disabled() {
        use std::os::unix::fs::PermissionsExt;

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
generate-stub = false
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("src/lib.mojo"),
            r#"
from std.python.bindings import PythonModuleBuilder

@export
def PyInit__native() -> PythonObject:
    var module = PythonModuleBuilder("_native")
    module.def_function[passthrough]("passthrough")
    return module.finalize()

def passthrough(value: PythonObject) raises -> PythonObject:
    return value
"#,
        )
        .unwrap();
        let mojo = temp.path().join("bin/mojo");
        fs::create_dir_all(mojo.parent().unwrap()).unwrap();
        fs::write(&mojo, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&mojo).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&mojo, permissions).unwrap();

        let config = ProjectConfig::load(temp.path()).unwrap();
        let output = temp
            .path()
            .join("python/demo/_native.cpython-311-darwin.so");

        super::compile_module_with_verbosity(
            &config,
            &config.modules[0],
            &output,
            &mojo,
            &[],
            Verbosity::default(),
        )
        .unwrap();

        assert!(!temp.path().join("python/demo/_native.pyi").exists());
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
