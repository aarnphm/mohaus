use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::config::{MojoModule, ProjectConfig};
use crate::error::{MohausError, Result};
use crate::python_info::PythonInfo;
use crate::toolchain::{resolve_verified_mojo, run_command};
use crate::wheel::write_file;

/// Ensure editable in-place extensions are built for a project.
///
/// # Errors
///
/// Returns an error when configuration loading, Mojo resolution, source hashing,
/// compilation, or sidecar writing fails.
pub fn ensure_editable_built(project_dir: impl AsRef<Path>, python: &PythonInfo) -> Result<()> {
    let config = ProjectConfig::load(project_dir.as_ref())?;
    let mojo = resolve_verified_mojo(&config.mojo_version)?;
    for module in &config.modules {
        let hash = source_hash(&config, module)?;
        let hash_path = editable_hash_path(&config, module);
        let extension_path = extension_output_path(&config, module, &python.ext_suffix);
        let current_hash = fs::read_to_string(&hash_path).ok();
        if extension_path.is_file() && current_hash.as_deref() == Some(hash.as_str()) {
            continue;
        }
        compile_module(&config, module, &extension_path, &mojo.executable)?;
        write_file(&hash_path, hash.as_bytes())?;
    }
    Ok(())
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
    let entry = config.project_dir.join(&module.entry);
    let mut args = vec![
        OsString::from("build"),
        entry.into_os_string(),
        OsString::from("--emit"),
        OsString::from("shared-lib"),
        OsString::from("-o"),
        output.as_os_str().to_os_string(),
    ];
    for include in &config.mojo_include_paths {
        args.push(OsString::from("-I"));
        args.push(config.project_dir.join(include).into_os_string());
    }
    for flag in &config.mojo_flags {
        args.push(OsString::from(flag));
    }
    run_command(mojo_executable, &args)
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
    use std::fs;

    use tempfile::TempDir;

    use crate::config::ProjectConfig;
    use crate::editable::source_hash;

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
}
