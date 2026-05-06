//! Integration tests that load real fixture projects from `tests/fixtures/*`
//! and assert the build pipeline carries `-I` paths through and produces a
//! valid argv for multi-module configs without touching the Mojo toolchain.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use mohaus_core::config::ProjectConfig;
use mohaus_core::editable::build_mojo_args;
use mohaus_core::stub::module_stub_plan_for_extension;

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .expect("workspace root must be two levels above mohaus-core")
        .to_path_buf()
}

fn copy_fixture(name: &str, into: &Path) {
    let source = workspace_root().join("tests").join("fixtures").join(name);
    copy_tree(&source, into);
}

fn copy_tree(source: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}

#[test]
fn editable_build_argv_carries_include_paths() {
    let root = tempfile::TempDir::new().unwrap();
    copy_fixture("with_include_paths", root.path());

    let config = ProjectConfig::load(root.path()).unwrap();
    assert_eq!(config.modules.len(), 1, "fixture should declare one module");

    let module = &config.modules[0];
    let output = root
        .path()
        .join("python")
        .join("include_demo")
        .join("_native.so");
    let args = build_mojo_args(&config, module, &output, None);

    let mut found_include = false;
    let expected_include = root.path().join("vendor");
    let dash_i: OsString = OsString::from("-I");
    for window in args.windows(2) {
        if window[0] == dash_i && window[1] == expected_include.as_os_str() {
            found_include = true;
            break;
        }
    }
    assert!(
        found_include,
        "expected -I {} in argv, got {:?}",
        expected_include.display(),
        args
    );
}

#[test]
fn editable_metadata_includes_dependencies_and_entry_points() {
    let root = tempfile::TempDir::new().unwrap();
    copy_fixture("with_include_paths", root.path());

    let config = ProjectConfig::load(root.path()).unwrap();
    let metadata = mohaus_core::wheel::metadata_text(&config, false);
    assert!(metadata.contains("Summary: Fixture with -I include paths"));
    assert!(metadata.contains("Requires-Dist: numpy>=1"));
    assert!(metadata.contains("Provides-Extra: dev"));
    assert!(metadata.contains("Project-URL: Source, https://example.com/include_demo"));
    assert!(metadata.contains("License-Expression: Apache-2.0"));
    assert!(metadata.contains("Author: Aaron <a@example.com>"));
}

#[test]
fn multi_module_fixture_produces_one_argv_per_module() {
    let root = tempfile::TempDir::new().unwrap();
    copy_fixture("multi_module", root.path());

    let config = ProjectConfig::load(root.path()).unwrap();
    assert_eq!(config.modules.len(), 2);

    let names = config
        .modules
        .iter()
        .map(|module| module.name.as_str().to_string())
        .collect::<Vec<_>>();
    assert!(names.contains(&"multi_module._native".to_string()));
    assert!(names.contains(&"multi_module._extra".to_string()));

    for module in &config.modules {
        let output = root.path().join(format!("{}.so", module.name.leaf()));
        let args = build_mojo_args(&config, module, &output, None);
        assert!(
            args.contains(&OsString::from("--emit")),
            "module {} missing --emit",
            module.name.as_str()
        );
        assert!(
            args.iter().any(|value| value == output.as_os_str()),
            "module {} missing -o {} in argv",
            module.name.as_str(),
            output.display()
        );
    }
}

#[test]
fn multi_module_fixture_produces_one_stub_per_targeted_binding() {
    let root = tempfile::TempDir::new().unwrap();
    copy_fixture("multi_module", root.path());

    let config = ProjectConfig::load(root.path()).unwrap();
    let stubs = config
        .modules
        .iter()
        .map(|module| {
            let extension = root.path().join(format!(
                "python/multi_module/{}.cpython-311-darwin.so",
                module.name.leaf()
            ));
            module_stub_plan_for_extension(&config, module, &extension).unwrap()
        })
        .collect::<Vec<_>>();

    assert_eq!(stubs.len(), 2);
    assert!(stubs.iter().any(|stub| {
        stub.path == root.path().join("python/multi_module/_native.pyi")
            && stub.text.contains("def hello(value: object) -> object")
    }));
    assert!(stubs.iter().any(|stub| {
        stub.path == root.path().join("python/multi_module/_extra.pyi")
            && stub.text.contains("def bonus(value: object) -> object")
    }));
}
