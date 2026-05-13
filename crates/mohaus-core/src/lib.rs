//! Core build logic for mohaus.

pub mod build;
pub mod config;
pub mod editable;
pub mod error;
pub mod log;
pub mod pyproject_edit;
pub mod python_info;
pub mod sdist;
pub mod stub;
pub mod toolchain;
pub mod wheel;

pub use build::{
    BuildOptions, EditableOptions, MetadataOptions, SdistOptions, build_editable_wheel,
    build_sdist, build_wheel, prepare_metadata_for_build_editable,
    prepare_metadata_for_build_wheel,
};
pub use config::{ModuleName, MojoVersion, PackageName, ProjectConfig};
pub use editable::{ensure_editable_built, ensure_editable_built_with_verbosity};
pub use error::{MohausError, Result};
pub use log::{VERBOSITY_ENV, Verbosity};
pub use python_info::{
    PythonInfo, discover_mojo_executable_from_python_scripts, discover_mojo_paths_from_python_roots,
};
