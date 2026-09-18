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
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{AttrStyle, Attribute, ItemMod, Meta, Token};

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

/// A lint attribute found in production code: whether it is inner (`#![...]`), whether
/// it is `allow` or `expect`, and the clippy lints it names.
struct LintAttribute {
    inner: bool,
    expect: bool,
    clippy_lints: Vec<String>,
}

/// Reads attributes as Rust syntax, through `syn`. Matching their written form missed
/// what the grammar and rustfmt allow — whitespace after `#!`, a `#[expect(...)]` split
/// over lines, `//` or `[` inside a string, several attributes in one `cfg_attr` — and
/// every miss was a way for an exception to escape the rule.
#[derive(Default)]
struct LintAttributes(Vec<LintAttribute>);

impl LintAttributes {
    fn of(source: &str) -> Result<Vec<LintAttribute>, syn::Error> {
        let mut found = Self::default();
        found.visit_file(&syn::parse_file(source)?);
        Ok(found.0)
    }

    /// `allow` and `expect` directly, or inside `cfg_attr(predicate, attr, attr, ...)`.
    fn collect(&mut self, meta: &Meta, inner: bool) {
        let Meta::List(list) = meta else {
            return;
        };
        let name = list.path.get_ident().map(ToString::to_string);
        let Ok(arguments) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        else {
            return;
        };
        match name.as_deref() {
            Some("cfg_attr") => {
                for attribute in arguments.iter().skip(1) {
                    self.collect(attribute, inner);
                }
            }
            Some(kind @ ("allow" | "expect")) => {
                let clippy_lints = arguments
                    .iter()
                    .filter_map(|argument| match argument {
                        // `reason = "..."` is a name-value pair; only paths are lints.
                        Meta::Path(path)
                            if path.segments.len() == 2
                                && path.segments.first().is_some_and(|s| s.ident == "clippy") =>
                        {
                            path.segments.last().map(|s| s.ident.to_string())
                        }
                        _ => None,
                    })
                    .collect();
                self.0.push(LintAttribute {
                    inner,
                    expect: kind == "expect",
                    clippy_lints,
                });
            }
            _ => {}
        }
    }
}

/// `#[cfg(test)]` on an item: clippy treats it as test code, and so does this rule.
fn is_test_only(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| match &attribute.meta {
        Meta::List(list) => list.path.is_ident("cfg") && list.tokens.to_string() == "test",
        _ => false,
    })
}

impl<'ast> Visit<'ast> for LintAttributes {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        self.collect(
            &attribute.meta,
            matches!(attribute.style, AttrStyle::Inner(_)),
        );
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if !is_test_only(&module.attrs) {
            visit::visit_item_mod(self, module);
        }
    }
}

#[test]
fn production_code_has_no_inner_lint_attribute() -> Result<(), Box<dyn Error>> {
    let files = production_files()?;
    assert!(
        files.len() > 10,
        "found only {} production files",
        files.len()
    );
    let mut offenders = Vec::new();
    for (path, text) in &files {
        for attribute in LintAttributes::of(text)? {
            if attribute.inner {
                offenders.push(path.clone());
            }
        }
    }
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
        for attribute in LintAttributes::of(&text)? {
            if attribute.expect {
                for lint in attribute.clippy_lints {
                    found.push((path.clone(), lint));
                }
            }
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

/// Each form that a textual scan got wrong, read correctly.
#[test]
fn lint_attributes_are_read_as_syntax() -> Result<(), Box<dyn Error>> {
    let cases: [(&str, bool, bool, &[&str]); 7] = [
        (
            "#![allow(clippy::unwrap_used)]",
            true,
            false,
            &["unwrap_used"],
        ),
        (
            "#!  [\n  allow(\n clippy::panic )]",
            true,
            false,
            &["panic"],
        ),
        (
            "#![cfg_attr(not(test), allow(clippy::panic))]",
            true,
            false,
            &["panic"],
        ),
        (
            "#[expect(\n    clippy::unwrap_used,\n    reason = \"x\"\n)]\nfn f() {}",
            false,
            true,
            &["unwrap_used"],
        ),
        (
            "const URL: &str = \"http://x\"; #[expect(clippy::panic, reason = \"y\")] fn f() {}",
            false,
            true,
            &["panic"],
        ),
        (
            "#[expect(clippy::unwrap_used, reason = \"a [ and clippy::panic in text\")] fn f() {}",
            false,
            true,
            &["unwrap_used"],
        ),
        (
            "#[cfg_attr(all(), expect(clippy::unwrap_used), expect(clippy::panic))] fn f() {}",
            false,
            true,
            &["unwrap_used", "panic"],
        ),
    ];
    for (source, inner, expect, lints) in cases {
        let found = LintAttributes::of(source)?;
        let named: Vec<&str> = found
            .iter()
            .flat_map(|a| a.clippy_lints.iter().map(String::as_str))
            .collect();
        assert_eq!(named, lints, "{source:?}");
        assert!(
            found.iter().all(|a| a.inner == inner && a.expect == expect),
            "{source:?}"
        );
    }
    // A test module is test code: its attributes are not production exceptions.
    assert!(LintAttributes::of("#[cfg(test)] mod tests { #![allow(clippy::panic)] }")?.is_empty());
    Ok(())
}
