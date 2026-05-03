use std::fs;
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::GzEncoder;
use tar::Builder;
use walkdir::WalkDir;

use crate::config::ProjectConfig;
use crate::error::{MohausError, Result};

/// Write an sdist archive for a project.
///
/// # Errors
///
/// Returns an error when files cannot be walked, read, or written into the
/// archive.
pub fn write_sdist_archive(config: &ProjectConfig, out_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(out_dir).map_err(|source| MohausError::CreateDir {
        path: out_dir.to_path_buf(),
        source,
    })?;
    let archive_name = format!("{}-{}.tar.gz", config.package.escaped(), config.version);
    let archive_path = out_dir.join(archive_name);
    let archive = fs::File::create(&archive_path).map_err(|source| MohausError::WriteFile {
        path: archive_path.clone(),
        source,
    })?;
    let encoder = GzEncoder::new(archive, Compression::default());
    let mut builder = Builder::new(encoder);
    let prefix = format!("{}-{}", config.package.escaped(), config.version);

    for path in sdist_files(config)? {
        let relative =
            path.strip_prefix(&config.project_dir)
                .map_err(|source| MohausError::Archive {
                    path: path.clone(),
                    source: std::io::Error::other(source.to_string()),
                })?;
        let archive_relative = Path::new(&prefix).join(relative);
        builder
            .append_path_with_name(&path, archive_relative)
            .map_err(|source| MohausError::Archive {
                path: archive_path.clone(),
                source,
            })?;
    }
    builder.finish().map_err(|source| MohausError::Archive {
        path: archive_path.clone(),
        source,
    })?;
    Ok(archive_path)
}

fn sdist_files(config: &ProjectConfig) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(&config.project_dir) {
        let entry = entry.map_err(|source| MohausError::WalkDir {
            path: config.project_dir.clone(),
            source,
        })?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if should_skip(path, &config.project_dir)? {
            continue;
        }
        files.push(path.to_path_buf());
    }
    files.sort();
    Ok(files)
}

fn should_skip(path: &Path, root: &Path) -> Result<bool> {
    let relative = path
        .strip_prefix(root)
        .map_err(|source| MohausError::Archive {
            path: path.to_path_buf(),
            source: std::io::Error::other(source.to_string()),
        })?;
    let mut components = relative.components();
    let first = components
        .next()
        .map(|component| component.as_os_str().to_string_lossy().to_string());
    let skip = first.as_deref().is_some_and(|name| {
        matches!(
            name,
            ".git" | ".venv" | "target" | "dist" | "__pycache__" | ".ruff_cache" | ".mohaus"
        )
    });
    Ok(skip)
}
