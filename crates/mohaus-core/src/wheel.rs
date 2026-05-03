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

use crate::config::ProjectConfig;
use crate::error::{MohausError, Result};

/// Write METADATA content for a project.
pub fn metadata_text(config: &ProjectConfig, editable: bool) -> String {
    let mut text = String::new();
    text.push_str("Metadata-Version: 2.3\n");
    text.push_str(&format!("Name: {}\n", config.package.as_str()));
    text.push_str(&format!("Version: {}\n", config.version));
    text.push_str(&format!("Requires-Python: {}\n", config.requires_python));
    if editable {
        text.push_str("Requires-Dist: mohaus>=0.1,<0.2\n");
    }
    text.push('\n');
    text
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
    Ok(dist_info)
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

/// Recursively copy one directory into another.
///
/// # Errors
///
/// Returns an error when the source tree cannot be walked or copied.
pub fn copy_dir(source_root: &Path, dest_root: &Path) -> Result<()> {
    for entry in WalkDir::new(source_root) {
        let entry = entry.map_err(|source| MohausError::WalkDir {
            path: source_root.to_path_buf(),
            source,
        })?;
        let source_path = entry.path();
        let relative =
            source_path
                .strip_prefix(source_root)
                .map_err(|source| MohausError::WheelMetadata {
                    message: format!("could not relativize {}: {source}", source_path.display()),
                })?;
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
