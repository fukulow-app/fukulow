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

/// Every `expect(clippy::…)` in production code, as `(path, lint)`. Startup, where
/// failing to start is correct, and the command line's own help text are the
/// exceptions the rule anticipates.
const PRODUCTION_CLIPPY_EXPECTATIONS: &[(&str, &str)] = &[
    // `fukulow help` prints its usage to standard output.
    ("crates/app/src/main.rs", "print_stdout"),
    // Bootstrap prompts, successful slug and fixed failure diagnostics.
    ("crates/app/src/bootstrap.rs", "print_stdout"),
    ("crates/app/src/bootstrap.rs", "print_stderr"),
];

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
        // Parsed, not compared as text: `#[cfg(test,)]` is the same predicate as
        // `#[cfg(test)]`, and `#[cfg(not(test))]` is not test code at all.
        Meta::List(list) if list.path.is_ident("cfg") => list
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .is_ok_and(|predicates| {
                predicates.len() == 1
                    && matches!(predicates.first(), Some(Meta::Path(path)) if path.is_ident("test"))
            }),
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
    // A test module is test code: its attributes are not production exceptions, however
    // the predicate is spelled. `not(test)` is production.
    for test_only in ["#[cfg(test)]", "#[cfg(test,)]", "#[cfg( test )]"] {
        let source = format!("{test_only} mod tests {{ #![allow(clippy::panic)] }}");
        assert!(LintAttributes::of(&source)?.is_empty(), "{source}");
    }
    assert_eq!(
        LintAttributes::of("#[cfg(not(test))] mod live { #![allow(clippy::panic)] }")?.len(),
        1
    );
    Ok(())
}

/// Router-building calls that register a route. Found as method calls
/// (`router.route(..)`) and as path calls (`Router::route(router, ..)`), since either
/// registers a route the sweep would never see.
const ROUTE_REGISTRATIONS: &[&str] = &[
    "route",
    "route_service",
    "nest",
    "nest_service",
    "merge",
    "fallback",
    "fallback_service",
];

#[derive(Default)]
struct RouteRegistrations(Vec<String>);

impl<'ast> Visit<'ast> for RouteRegistrations {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let name = call.method.to_string();
        if ROUTE_REGISTRATIONS.contains(&name.as_str()) {
            self.0.push(format!(".{name}(…)"));
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(function) = &*call.func {
            let segments = &function.path.segments;
            if let Some(last) = segments.last().filter(|_| segments.len() > 1) {
                let name = last.ident.to_string();
                if ROUTE_REGISTRATIONS.contains(&name.as_str()) {
                    self.0.push(format!("…::{name}(…)"));
                }
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if !is_test_only(&module.attrs) {
            visit::visit_item_mod(self, module);
        }
    }
}

#[test]
fn routes_are_registered_only_in_the_registry() -> Result<(), Box<dyn Error>> {
    let mut offenders = Vec::new();
    let mut registry = None;
    for (path, text) in production_files()? {
        if !path.starts_with("crates/app/src/") {
            continue;
        }
        let mut calls = RouteRegistrations::default();
        calls.visit_file(&syn::parse_file(&text)?);
        if path == "crates/app/src/route_registry.rs" {
            registry = Some(calls.0);
        } else {
            offenders.extend(calls.0.into_iter().map(|call| format!("{path}: {call}")));
        }
    }
    assert!(
        offenders.is_empty(),
        "route registration outside route_registry.rs: {offenders:?}"
    );
    // The registry itself registers exactly once: the fold over ROUTES. A second call
    // there would add a route the table does not list.
    assert_eq!(
        registry,
        Some(vec![".route(…)".to_owned()]),
        "route_registry.rs must register routes only by folding ROUTES"
    );
    Ok(())
}

#[test]
fn route_registrations_are_found_in_every_calling_form() -> Result<(), Box<dyn Error>> {
    // Written out here rather than read from ROUTE_REGISTRATIONS, so dropping a name from
    // that list fails this test instead of silently shrinking what it checks.
    for name in [
        "route",
        "route_service",
        "nest",
        "nest_service",
        "merge",
        "fallback",
        "fallback_service",
    ] {
        for source in [
            format!("fn f(r: Router) -> Router {{ r.{name}(x) }}"),
            format!("fn f(r: Router) -> Router {{ r\n    .{name}(x) }}"),
            format!("fn f(r: Router) -> Router {{ Router::{name}(r, x) }}"),
            format!("fn f(r: Router) -> Router {{ axum::Router::{name}(r, x) }}"),
        ] {
            let mut calls = RouteRegistrations::default();
            calls.visit_file(&syn::parse_file(&source)?);
            assert_eq!(calls.0.len(), 1, "{source:?}");
        }
    }
    let mut calls = RouteRegistrations::default();
    calls.visit_file(&syn::parse_file(
        "#[cfg(test)] mod tests { fn f(r: Router) -> Router { r.route(\"/x\", get(h)) } }",
    )?);
    assert!(
        calls.0.is_empty(),
        "a test module's router is not production"
    );
    Ok(())
}

/// The `std::env` functions that read a variable by name.
const ENV_READERS: &[&str] = &["var", "var_os", "vars", "vars_os"];

/// `Some(name)` when `path` names a `std::env` reader through its module, such as
/// `env::var` or `std::env::var_os`. A bare `var` can only come from an import, which
/// is refused, so the module segment is enough.
fn env_reader(path: &syn::Path) -> Option<String> {
    let mut names = path.segments.iter().rev().map(|s| s.ident.to_string());
    let name = names.next()?;
    (names.next().as_deref() == Some("env") && ENV_READERS.contains(&name.as_str())).then_some(name)
}

/// Where production code reads the migrator's URL, embeds migrations, or calls
/// `db::migrate`, as `(file, form)`.
#[derive(Default)]
struct MigratorUses {
    file: String,
    found: Vec<(String, &'static str)>,
}

impl<'ast> Visit<'ast> for MigratorUses {
    // `env::var("NAME")` and `env::var_os("NAME")` are the only accepted readers: called
    // directly, with the name as a string literal, so the rule reads which variable it
    // is. Any other reference to a reader is refused in `visit_expr_path`.
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        let direct = match &*call.func {
            syn::Expr::Path(function) if function.qself.is_none() => {
                env_reader(&function.path).filter(|name| name == "var" || name == "var_os")
            }
            _ => None,
        };
        if direct.is_none() {
            visit::visit_expr_call(self, call);
            return;
        }
        match call.args.iter().collect::<Vec<_>>().as_slice() {
            [
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(name),
                    ..
                }),
            ] => {
                if name.value().contains("MIGRATOR") {
                    self.found.push((self.file.clone(), "env"));
                }
            }
            // A computed name could be assembled into the migrator's variable.
            _ => self.found.push((self.file.clone(), "env name not literal")),
        }
        for argument in &call.args {
            self.visit_expr(argument);
        }
    }

    // Reached only by a reader that is not called directly: `let read = env::var;`,
    // `.map(env::var_os)`, or `env::vars()`, which reads every variable at once.
    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        match env_reader(&path.path).as_deref() {
            Some("vars" | "vars_os") => self.found.push((self.file.clone(), "env vars")),
            Some(_) => self.found.push((self.file.clone(), "env reader value")),
            None => {}
        }
        visit::visit_expr_path(self, path);
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        self.extern_crate(item);
        visit::visit_item_extern_crate(self, item);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac
            .path
            .segments
            .last()
            .is_some_and(|s| s.ident == "migrate")
        {
            self.found.push((self.file.clone(), "migrate!"));
        }
        visit::visit_macro(self, mac);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let names: Vec<_> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        if names.windows(2).any(|pair| pair == ["db", "migrate"]) {
            self.found.push((self.file.clone(), "db::migrate"));
        }
        visit::visit_path(self, path);
    }

    // `use db::migrate` and `use db::{migrate as m}` are use trees, not paths.
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        fn imports_migrate(tree: &syn::UseTree, under_db: bool) -> bool {
            match tree {
                syn::UseTree::Path(path) => imports_migrate(&path.tree, path.ident == "db"),
                syn::UseTree::Group(group) => group
                    .items
                    .iter()
                    .any(|tree| imports_migrate(tree, under_db)),
                syn::UseTree::Name(name) => under_db && name.ident == "migrate",
                syn::UseTree::Rename(rename) => under_db && rename.ident == "migrate",
                syn::UseTree::Glob(_) => under_db,
            }
        }
        // `use db as database;` would let `database::migrate` pass unseen, so renaming
        // the crate is refused outright rather than followed.
        fn aliases_db(tree: &syn::UseTree, under_db: bool) -> bool {
            match tree {
                syn::UseTree::Path(path) => path.ident == "db" && aliases_db(&path.tree, true),
                syn::UseTree::Group(group) => {
                    group.items.iter().any(|tree| aliases_db(tree, under_db))
                }
                syn::UseTree::Rename(rename) => {
                    if under_db {
                        rename.ident == "self"
                    } else {
                        rename.ident == "db"
                    }
                }
                syn::UseTree::Name(_) | syn::UseTree::Glob(_) => false,
            }
        }
        if imports_migrate(&item.tree, false) {
            self.found.push((self.file.clone(), "db::migrate"));
        }
        if aliases_db(&item.tree, false) {
            self.found.push((self.file.clone(), "db alias"));
        }
        // Readers are recognised as `env::var` and `env::var_os`. Importing a reader
        // (`use std::env::var;`, a glob) would leave a bare `var`, and renaming one or the
        // module (`use std::env as e;`) would hide it, so both are refused in production.
        fn env_import(tree: &syn::UseTree, under_env: bool) -> Option<&'static str> {
            match tree {
                syn::UseTree::Path(path) => {
                    env_import(&path.tree, under_env || path.ident == "env")
                }
                syn::UseTree::Group(group) => group
                    .items
                    .iter()
                    .find_map(|tree| env_import(tree, under_env)),
                syn::UseTree::Rename(rename) => (rename.ident == "env"
                    || (under_env
                        && (rename.ident == "self"
                            || ENV_READERS.iter().any(|name| rename.ident == name))))
                .then_some("env alias"),
                syn::UseTree::Name(name) => (under_env
                    && ENV_READERS.iter().any(|reader| name.ident == reader))
                .then_some("env import"),
                syn::UseTree::Glob(_) => under_env.then_some("env import"),
            }
        }
        if let Some(form) = env_import(&item.tree, false) {
            self.found.push((self.file.clone(), form));
        }
        // `use sqlx::migrate as embed;` would make `embed!` embed migrations unseen, since
        // the macro is found by its last segment. Renaming `migrate` is refused outside
        // `db`, where `use db::{migrate as …}` is already a `db::migrate` import.
        fn renames_migrate(tree: &syn::UseTree, under_db: bool) -> bool {
            match tree {
                syn::UseTree::Path(path) => renames_migrate(&path.tree, path.ident == "db"),
                syn::UseTree::Group(group) => group
                    .items
                    .iter()
                    .any(|tree| renames_migrate(tree, under_db)),
                syn::UseTree::Rename(rename) => !under_db && rename.ident == "migrate",
                syn::UseTree::Name(_) | syn::UseTree::Glob(_) => false,
            }
        }
        if renames_migrate(&item.tree, false) {
            self.found.push((self.file.clone(), "migrate alias"));
        }
        visit::visit_item_use(self, item);
    }
}

impl MigratorUses {
    fn extern_crate(&mut self, item: &syn::ItemExternCrate) {
        if item.ident == "db" && item.rename.is_some() {
            self.found.push((self.file.clone(), "db alias"));
        }
    }
}

fn migrator_uses(file: &str, source: &str) -> Result<Vec<(String, &'static str)>, syn::Error> {
    let mut uses = MigratorUses {
        file: file.to_owned(),
        ..MigratorUses::default()
    };
    uses.visit_file(&syn::parse_file(source)?);
    Ok(uses.found)
}

/// The server never runs migrations and never holds the migrator's credentials:
/// only `fukulow migrate` reads `MIGRATOR_DATABASE_URL` and calls `db::migrate`,
/// and only `db`'s migrations module embeds them.
#[test]
fn only_the_migrate_command_touches_the_migrator() -> Result<(), Box<dyn Error>> {
    let mut found = Vec::new();
    for file in ModuleTree::of_workspace()?.production {
        let relative = file
            .strip_prefix(workspace())?
            .to_string_lossy()
            .replace('\\', "/");
        found.extend(migrator_uses(&relative, &fs::read_to_string(file)?)?);
    }
    found.sort();
    assert_eq!(
        found,
        [
            ("crates/app/src/migrate.rs".to_owned(), "db::migrate"),
            ("crates/app/src/migrate.rs".to_owned(), "env"),
            ("crates/db/src/migrations.rs".to_owned(), "migrate!"),
        ]
    );
    Ok(())
}

#[test]
fn migrator_uses_are_found_in_every_form() -> Result<(), Box<dyn Error>> {
    for (source, form) in [
        (
            r#"fn f() { let _ = std::env::var("MIGRATOR_DATABASE_URL"); }"#,
            "env",
        ),
        (
            r#"fn f() { let _ = env::var_os("MIGRATOR_DATABASE_URL"); }"#,
            "env",
        ),
        // A bare `var` is caught at its import, the only way it can name the reader.
        (
            r#"use std::env::var; fn f() { let _ = var("MIGRATOR_DATABASE_URL"); }"#,
            "env import",
        ),
        (
            r#"fn f() { let _ = sqlx::migrate!("../../migrations"); }"#,
            "migrate!",
        ),
        (
            "async fn f() { let _ = db::migrate(url).await; }",
            "db::migrate",
        ),
        ("use db::migrate;", "db::migrate"),
        ("use db::{connect, migrate as run};", "db::migrate"),
        ("use db::*;", "db::migrate"),
        // An alias of the crate is refused, since its calls would not read `db::migrate`.
        ("use db as database;", "db alias"),
        ("use ::db as database;", "db alias"),
        ("use db::{self as database};", "db alias"),
        ("use {db as database};", "db alias"),
        ("extern crate db as database;", "db alias"),
        // Renaming a reader or the module would hide a call from the rule.
        ("use std::env::var as read_env;", "env alias"),
        ("use std::env::var_os as read_env;", "env alias"),
        ("use std::{env::{var_os as read_env}};", "env alias"),
        ("use env::var as read_env;", "env alias"),
        ("use ::std::env::vars as all_env;", "env alias"),
        ("use std::env as environment;", "env alias"),
        ("use std::{env as environment};", "env alias"),
        ("use std::env::{self as environment};", "env alias"),
        // Importing a reader would leave a bare `var` the rule cannot tell from others.
        ("use std::env::var;", "env import"),
        ("use std::env::{self, var_os};", "env import"),
        ("use std::env::*;", "env import"),
        // A reader taken as a function value, or given a computed name.
        ("fn f() { let read = std::env::var; }", "env reader value"),
        ("fn f() { let read = env::var_os; }", "env reader value"),
        (
            "fn f() { let _ = names.iter().map(env::var); }",
            "env reader value",
        ),
        ("fn f() { let _ = env::var(name); }", "env name not literal"),
        (
            r#"fn f() { let _ = env::var_os(format!("{}_URL", prefix)); }"#,
            "env name not literal",
        ),
        // Renaming the migration macro would hide `migrate!`.
        ("use sqlx::migrate as embed;", "migrate alias"),
        ("use ::sqlx::{migrate as embed};", "migrate alias"),
        ("use sqlx::{self, migrate as embed};", "migrate alias"),
        // Reading every variable at once includes the migrator's.
        ("fn f() { let _ = std::env::vars(); }", "env vars"),
        ("fn f() { for _ in env::vars_os() {} }", "env vars"),
    ] {
        assert_eq!(
            migrator_uses("fixture.rs", source)?,
            [("fixture.rs".to_owned(), form)],
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn ordinary_uses_are_not_migrator_uses() -> Result<(), Box<dyn Error>> {
    for source in [
        r#"fn f() { let _ = std::env::var("DATABASE_URL"); }"#,
        r#"const USAGE: &str = "never MIGRATOR_DATABASE_URL";"#,
        "use db::connect as open;",
        "use other::db as unrelated;",
        "use std::env;",
        "use std::env::{self, VarError};",
        "use std::env::args_os as arguments;",
        "use other::var as unrelated;",
        r#"fn f() { let _ = env::var("DATABASE_URL"); let _ = std::env::var_os("RUST_LOG"); }"#,
        "fn f() { let _ = std::env::args_os(); let _ = other::var(name); }",
        "fn f() { let read = settings::var; }",
        "use sqlx::migrate::Migrator;",
        "use sqlx::migrate::MigrateError as SqlxMigrateError;",
    ] {
        assert!(migrator_uses("fixture.rs", source)?.is_empty(), "{source}");
    }
    Ok(())
}
