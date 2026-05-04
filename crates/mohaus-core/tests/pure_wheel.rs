//! Verifies that `[tool.mohaus] pure = true` produces a valid wheel without
//! requiring a Mojo toolchain. mohaus-mojo's `mohaus.backend` build path
//! depends on this contract.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;

use mohaus_core::config::ProjectConfig;
use mohaus_core::python_info::PythonInfo;
use mohaus_core::{BuildOptions, build_wheel};

fn fixture(root: &std::path::Path) {
    fs::write(
        root.join("README.md"),
        "# pure_demo\n\nFixture for the pure-mode wheel test.\n",
    )
    .unwrap();
    fs::write(
        root.join("LICENSE"),
        "Apache License 2.0 placeholder for tests.\n",
    )
    .unwrap();
    let pyproject = r#"[build-system]
requires = ["mohaus>=0.1,<0.2"]
build-backend = "mohaus.backend"

[project]
name = "pure-demo"
version = "0.1.0"
description = "Pure-mode parity smoke."
readme = "README.md"
license = "Apache-2.0"
license-files = ["LICENSE"]

[tool.mohaus]
pure = true
python-src = "src"
"#;
    fs::write(root.join("pyproject.toml"), pyproject).unwrap();
    let pkg = root.join("src").join("pure_demo");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("__init__.py"), "VERSION = \"0.1.0\"\n").unwrap();
    fs::write(pkg.join("py.typed"), "").unwrap();
    let mojopkg_dir = pkg.join("_mojopkg");
    fs::create_dir_all(&mojopkg_dir).unwrap();
    fs::write(mojopkg_dir.join("fake.mojopkg"), b"\x00mojopkg-bytes").unwrap();
}

#[test]
fn pure_mode_builds_a_wheel_without_mojo_toolchain() {
    let temp = tempfile::TempDir::new().unwrap();
    fixture(temp.path());

    let config = ProjectConfig::load(temp.path()).unwrap();
    assert!(config.pure);
    assert!(config.modules.is_empty());
    assert!(config.mojo_version.is_none());

    let out_dir = temp.path().join("dist");
    let python = PythonInfo::from_parts(
        ".cpython-311-darwin.so".to_string(),
        "cpython-311".to_string(),
        "macosx-11.0-arm64".to_string(),
        3,
        11,
    )
    .unwrap();
    let wheel = build_wheel(&BuildOptions {
        project_dir: temp.path().to_path_buf(),
        out_dir,
        python,
        release: false,
        metadata_dir: None,
    })
    .unwrap();
    assert!(wheel.is_file());
    let wheel_name = wheel.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        wheel_name.contains("py3-none-any"),
        "wheel name should carry the pure tag, got {wheel_name}"
    );
    assert!(wheel_name.starts_with("pure_demo-0.1.0-"));
}

#[test]
fn pure_mode_rejects_module_name_field() {
    let temp = tempfile::TempDir::new().unwrap();
    fixture(temp.path());
    let pyproject = r#"[build-system]
requires = ["mohaus>=0.1,<0.2"]
build-backend = "mohaus.backend"

[project]
name = "pure-demo"
version = "0.1.0"

[tool.mohaus]
pure = true
python-src = "src"
module-name = "pure_demo._native"
"#;
    fs::write(temp.path().join("pyproject.toml"), pyproject).unwrap();
    let error = ProjectConfig::load(temp.path()).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("pure = true"),
        "expected pure-mode error, got {message}"
    );
}

#[test]
fn pure_mode_with_missing_mojo_version_succeeds() {
    let temp = tempfile::TempDir::new().unwrap();
    fixture(temp.path());
    // No `.mojo-version` file written. Pure mode should still load.
    let config = ProjectConfig::load(temp.path()).unwrap();
    assert!(config.pure);
    assert_eq!(config.modules.len(), 0);
    let _ = config.project_dir;
}
