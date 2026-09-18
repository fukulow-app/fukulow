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

/// Where a module's files live, following Rust's module tree from a crate root.
struct ModuleTree {
    production: Vec<PathBuf>,
    test_only: Vec<PathBuf>,
}

impl ModuleTree {
    /// Walks every crate root under `crates/*/src` (`lib.rs`, `main.rs`) through its
    /// `mod` declarations. A module declared under `#[cfg(test)]` — directly or through
    /// an enclosing inline module — is test code, as clippy treats it; everything else
    /// reached is production. Test-ness comes from the tree, not from a file's name: a
    /// production `mod foo_tests;` is production (#51).
    fn of_workspace() -> Result<Self, Box<dyn Error>> {
        let mut tree = Self {
            production: Vec::new(),
            test_only: Vec::new(),
        };
        for crate_dir in fs::read_dir(workspace().join("crates"))? {
            let src = crate_dir?.path().join("src");
            for root in ["lib.rs", "main.rs"] {
                let root = src.join(root);
                if root.is_file() {
                    tree.walk_file(&root, &src, false)?;
                }
            }
        }
        Ok(tree)
    }

    /// `dir` is where this file's child modules live: the file's own directory for a
    /// crate root or `mod.rs`, and `<dir>/<stem>/` for any other `<stem>.rs`.
    fn walk_file(
        &mut self,
        file: &Path,
        dir: &Path,
        test_only: bool,
    ) -> Result<(), Box<dyn Error>> {
        if test_only {
            self.test_only.push(file.to_owned());
        } else {
            self.production.push(file.to_owned());
        }
        let parsed = syn::parse_file(&fs::read_to_string(file)?)?;
        self.walk_items(&parsed.items, file, dir, test_only)
    }

    fn walk_items(
        &mut self,
        items: &[syn::Item],
        file: &Path,
        dir: &Path,
        test_only: bool,
    ) -> Result<(), Box<dyn Error>> {
        for item in items {
            let syn::Item::Mod(module) = item else {
                continue;
            };
            let test_only = test_only || is_test_only(&module.attrs);
            let name = module.ident.to_string();
            match &module.content {
                // An inline module's own children live one directory further down.
                Some((_, items)) => self.walk_items(items, file, &dir.join(&name), test_only)?,
                None => {
                    let (child, child_dir) = Self::resolve(module, file, dir, &name)?;
                    self.walk_file(&child, &child_dir, test_only)?;
                }
            }
        }
        Ok(())
    }

    fn resolve(
        module: &ItemMod,
        file: &Path,
        dir: &Path,
        name: &str,
    ) -> Result<(PathBuf, PathBuf), Box<dyn Error>> {
        // `#[path = "…"]` is relative to the declaring file's directory.
        for attribute in &module.attrs {
            if let Meta::NameValue(pair) = &attribute.meta
                && pair.path.is_ident("path")
                && let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(path),
                    ..
                }) = &pair.value
            {
                let parent = file.parent().ok_or("module file has no directory")?;
                let child = parent.join(path.value());
                // A `mod.rs` keeps its children beside it; any other file, in `<stem>/`.
                let child_dir = if child.file_name().is_some_and(|name| name == "mod.rs") {
                    child
                        .parent()
                        .ok_or("module file has no directory")?
                        .to_owned()
                } else {
                    child.with_extension("")
                };
                return Ok((child, child_dir));
            }
        }
        let flat = dir.join(format!("{name}.rs"));
        if flat.is_file() {
            return Ok((flat, dir.join(name)));
        }
        let nested = dir.join(name).join("mod.rs");
        if nested.is_file() {
            return Ok((nested, dir.join(name)));
        }
        Err(format!("module `{name}` declared in {} has no file", file.display()).into())
    }
}

/// Production source: every file the module tree reaches without passing through a
/// `#[cfg(test)]` module.
fn production_files() -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut files = Vec::new();
    for path in ModuleTree::of_workspace()?.production {
        let relative = path
            .strip_prefix(workspace())?
            .to_string_lossy()
            .replace('\\', "/");
        files.push((relative, fs::read_to_string(&path)?));
    }
    files.sort();
    Ok(files)
}

/// Every `.rs` file under `crates/*/src` must be reached by the module tree, as
/// production or as test code. A file the walk failed to reach would otherwise escape the
/// rules without anything saying so.
#[test]
fn every_source_file_is_classified_by_the_module_tree() -> Result<(), Box<dyn Error>> {
    let tree = ModuleTree::of_workspace()?;
    let mut classified: Vec<PathBuf> = tree
        .production
        .iter()
        .chain(&tree.test_only)
        .map(fs::canonicalize)
        .collect::<Result<_, _>>()?;
    classified.sort();
    let mut unreached = Vec::new();
    let mut pending: Vec<PathBuf> = fs::read_dir(workspace().join("crates"))?
        .map(|entry| entry.map(|e| e.path().join("src")))
        .collect::<Result<_, _>>()?;
    while let Some(dir) = pending.pop() {
        if !dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && classified.binary_search(&fs::canonicalize(&path)?).is_err()
            {
                unreached.push(path.display().to_string());
            }
        }
    }
    assert!(
        unreached.is_empty(),
        "source files outside the module tree: {unreached:?}"
    );
    assert!(
        !tree.test_only.is_empty(),
        "no test-only module found; the #[cfg(test)] detection may be broken"
    );
    Ok(())
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
