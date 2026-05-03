use std::path::PathBuf;

use thiserror::Error;

/// Result alias for mohaus core operations.
pub type Result<T> = std::result::Result<T, MohausError>;

/// Structured errors emitted by mohaus core.
#[derive(Debug, Error)]
pub enum MohausError {
    #[error("could not read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("could not write {path}: {source}")]
    WriteFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("could not create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("could not copy {source_path} to {dest_path}: {source}")]
    CopyFile {
        source_path: PathBuf,
        dest_path: PathBuf,
        source: std::io::Error,
    },

    #[error("invalid pyproject.toml at {path}: {source}")]
    InvalidToml {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("invalid mohaus project: {message}")]
    InvalidProject { message: String },

    #[error("invalid package name `{value}`: {message}")]
    InvalidPackageName { value: String, message: String },

    #[error("invalid module name `{value}`: {message}")]
    InvalidModuleName { value: String, message: String },

    #[error("invalid Mojo version `{value}`: {message}")]
    InvalidMojoVersion { value: String, message: String },

    #[error(
        "could not find a Mojo executable; searched $MOHAUS_MOJO, $PATH, and $MODULAR_HOME/bin/mojo"
    )]
    MissingMojo,

    #[error(
        "Mojo version mismatch: project pins `{expected}`, but `{executable}` reported `{actual}`"
    )]
    MojoVersionMismatch {
        expected: String,
        actual: String,
        executable: PathBuf,
    },

    #[error("failed to run `{program}`: {source}")]
    CommandIo {
        program: String,
        source: std::io::Error,
    },

    #[error("command `{program}` failed with status {status}: {stderr}")]
    CommandFailed {
        program: String,
        status: String,
        stderr: String,
    },

    #[error("wheel metadata failed: {message}")]
    WheelMetadata { message: String },

    #[error("archive creation failed for {path}: {source}")]
    Archive {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("zip creation failed for {path}: {source}")]
    Zip {
        path: PathBuf,
        source: zip::result::ZipError,
    },

    #[error("walkdir failed at {path}: {source}")]
    WalkDir {
        path: PathBuf,
        source: walkdir::Error,
    },
}
