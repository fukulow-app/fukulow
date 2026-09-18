//! Rules about production source that the compiler and clippy cannot see.
//!
//! Clippy's `allow_attributes` refuses an outer `#[allow]` but not an inner `#![allow]`,
//! so a module or crate could silence the production lints wholesale without anything
//! failing. And `allow_attributes_without_reason` only checks that a reason is present.
//! These tests close both: no inner lint attribute in production code, and every
//! production `expect(clippy::…)` is on one list, so adding an exception is always an
//! edit to that list where its reason is read.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

/// Every `expect(clippy::…)` in production code, as `(path, lint)`. Empty on purpose:
/// the only exception the rule anticipates is startup, where failing to start is
/// correct, and there is none yet.
const PRODUCTION_CLIPPY_EXPECTATIONS: &[(&str, &str)] = &[];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Production source: every `.rs` file under `crates/*/src`, except files that are test
/// modules by name (`tests.rs`, `*_tests.rs`), which clippy treats as tests too.
fn production_files() -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut files = Vec::new();
    let mut pending = Vec::new();
    for crate_dir in fs::read_dir(workspace().join("crates"))? {
        let src = crate_dir?.path().join("src");
        if src.is_dir() {
            pending.push(src);
        }
    }
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if path.is_dir() {
                pending.push(path);
            } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
                let relative = path
                    .strip_prefix(workspace())?
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push((relative, fs::read_to_string(&path)?));
            }
        }
    }
    files.sort();
    Ok(files)
}

#[test]
fn production_code_has_no_inner_lint_attribute() -> Result<(), Box<dyn Error>> {
    let files = production_files()?;
    assert!(
        files.len() > 10,
        "found only {} production files",
        files.len()
    );
    let offenders: Vec<String> = files
        .iter()
        .flat_map(|(path, text)| {
            text.lines().enumerate().filter_map(move |(index, line)| {
                let line = line.trim_start();
                (line.starts_with("#![allow(") || line.starts_with("#![expect("))
                    .then(|| format!("{path}:{}", index + 1))
            })
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "inner lint attributes in production code: {offenders:?}"
    );
    Ok(())
}

#[test]
fn production_clippy_expectations_are_all_listed() -> Result<(), Box<dyn Error>> {
    let mut found: Vec<(String, String)> = Vec::new();
    for (path, text) in production_files()? {
        for part in text.split("expect(clippy::").skip(1) {
            let lint: String = part
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            found.push((path.clone(), lint));
        }
    }
    found.sort();
    let mut listed: Vec<(String, String)> = PRODUCTION_CLIPPY_EXPECTATIONS
        .iter()
        .map(|(path, lint)| ((*path).to_owned(), (*lint).to_owned()))
        .collect();
    listed.sort();
    assert_eq!(
        found, listed,
        "production `expect(clippy::…)` sites must equal PRODUCTION_CLIPPY_EXPECTATIONS"
    );
    Ok(())
}
