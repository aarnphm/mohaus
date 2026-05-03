//! Core build logic for mohaus.

pub mod build;
pub mod config;
pub mod editable;
pub mod error;
pub mod python_info;
pub mod sdist;
pub mod toolchain;
pub mod wheel;

pub use build::{
    BuildOptions, EditableOptions, MetadataOptions, SdistOptions, build_editable_wheel,
    build_sdist, build_wheel, prepare_metadata_for_build_editable,
    prepare_metadata_for_build_wheel,
};
pub use config::{ModuleName, MojoVersion, PackageName, ProjectConfig};
pub use editable::ensure_editable_built;
pub use error::{MohausError, Result};
pub use python_info::PythonInfo;

/// Mojo package version used by generated v1 projects.
pub const DEFAULT_MOJO_VERSION: &str = "1.0.0b2.dev2026050306";
