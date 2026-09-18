//! Rules about production source that the compiler and clippy cannot see.
//!
//! Clippy's `allow_attributes` refuses an outer `#[allow]` but not an inner `#![allow]`,
//! so a module or crate could silence the production lints wholesale without anything
//! failing. And `allow_attributes_without_reason` only checks that a reason is present.
//! These tests close both: no inner lint attribute in production code, and every
//! production `expect(clippy::…)` is on one list, so adding an exception is always an
//! edit to that list where its reason is read.
//! Role checks likewise need one capability table: a second comparison or spelling
//! would silently bypass changes to that table.

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

// File, logical module path, inherent/trait impl and function all participate in the
// identity. A #[path] module or a same-named method must not borrow an exception.
const ROLE_COMPARISON_EXCEPTIONS: &[(&str, &str)] = &[
    (
        "crates/db/src/operations/membership.rs::operations::membership::change_member_role",
        "Demoting an active owner must preserve the last active owner",
    ),
    (
        "crates/db/src/operations/membership.rs::operations::membership::suspend_member",
        "Suspending an owner must preserve the last active owner",
    ),
];

const ROLE_LITERAL_EXCEPTIONS: &[(&str, &str, &str)] = &[
    (
        "crates/db/src/audit.rs::audit::impl<Event>::invite_created",
        "organization",
        "Audit target_type identifies the target entity, not a channel scope",
    ),
    (
        "crates/db/src/audit.rs::audit::impl<Event>::invite_revoked",
        "organization",
        "Audit target_type identifies the target entity, not a channel scope",
    ),
    (
        "crates/db/src/audit.rs::audit::impl<Event>::organization_created",
        "organization",
        "Audit target_type identifies the target entity, not a channel scope",
    ),
    (
        "crates/db/src/audit.rs::audit::impl<Event>::team_created",
        "team",
        "Audit target_type identifies the target entity, not a channel scope",
    ),
];

const CAPABILITY_TABLE: &str = "crates/domain/src/lib.rs::impl<OrganizationRole>::holds";
const SCOPE_SPELLINGS: &str = "crates/domain/src/lib.rs::impl<ChannelScope>::as_str";

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Where a module's files live, following Rust's module tree from a crate root.
struct ModuleTree {
    production: Vec<PathBuf>,
    test_only: Vec<PathBuf>,
    production_modules: Vec<(PathBuf, Vec<String>)>,
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
            production_modules: Vec::new(),
        };
        for crate_dir in fs::read_dir(workspace().join("crates"))? {
            let src = crate_dir?.path().join("src");
            for root in ["lib.rs", "main.rs"] {
                let root = src.join(root);
                if root.is_file() {
                    tree.walk_file(&root, &src, false, &[])?;
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
        modules: &[String],
    ) -> Result<(), Box<dyn Error>> {
        if test_only {
            self.test_only.push(file.to_owned());
        } else {
            self.production.push(file.to_owned());
            self.production_modules
                .push((file.to_owned(), modules.to_vec()));
        }
        let parsed = syn::parse_file(&fs::read_to_string(file)?)?;
        self.walk_items(&parsed.items, file, dir, test_only, modules)
    }

    fn walk_items(
        &mut self,
        items: &[syn::Item],
        file: &Path,
        dir: &Path,
        test_only: bool,
        modules: &[String],
    ) -> Result<(), Box<dyn Error>> {
        for item in items {
            let syn::Item::Mod(module) = item else {
                continue;
            };
            let test_only = test_only || is_test_only(&module.attrs);
            let name = module.ident.to_string();
            let mut child_modules = modules.to_vec();
            child_modules.push(name.clone());
            match &module.content {
                // An inline module's own children live one directory further down.
                Some((_, items)) => {
                    self.walk_items(items, file, &dir.join(&name), test_only, &child_modules)?
                }
                None => {
                    let (child, child_dir) = Self::resolve(module, file, dir, &name)?;
                    self.walk_file(&child, &child_dir, test_only, &child_modules)?;
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

#[derive(Debug, PartialEq, Eq)]
struct RoleViolation {
    identity: String,
    form: &'static str,
}

struct RoleRules<'a> {
    file: &'a str,
    modules: Vec<String>,
    implementation: Option<String>,
    self_type: Option<String>,
    function: Option<String>,
    // Nested items retain their lexical identity, but never their parent's exemption.
    enclosing: Vec<String>,
    violations: Vec<RoleViolation>,
    used_comparisons: Vec<String>,
    used_literals: Vec<(String, String)>,
    scopes: Vec<String>,
}

fn path_name(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn type_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(ty)
            if ty.qself.is_none()
                && ty
                    .path
                    .segments
                    .iter()
                    .all(|s| matches!(s.arguments, syn::PathArguments::None)) =>
        {
            path_name(&ty.path)
        }
        // No existing exception names a compound type. Keep it distinct from the
        // inherent impls on named types, without needing syn's printing feature.
        _ => "<compound type>".to_owned(),
    }
}

fn is_role_type(name: &str) -> bool {
    matches!(name, "OrganizationRole" | "TeamRole")
}

// A variant is recognised by its UpperCamelCase shape, not by a list of today's
// names: a variant added later is refused without editing this rule. Methods are
// snake_case and associated constants (`ALL`) are SCREAMING_CASE, so neither matches.
fn is_variant_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.chars().any(|c| c.is_ascii_lowercase())
}

// The spellings `ChannelScope::as_str` returns, read from its source so that a scope
// added later is reserved without editing this rule.
fn scope_spellings() -> Result<Vec<String>, syn::Error> {
    struct AsStr(Vec<String>, bool);
    impl<'ast> Visit<'ast> for AsStr {
        fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
            let scope = type_name(&item.self_ty) == "ChannelScope" && item.trait_.is_none();
            for item in &item.items {
                if let syn::ImplItem::Fn(function) = item {
                    self.1 = scope && function.sig.ident == "as_str";
                    visit::visit_impl_item_fn(self, function);
                    self.1 = false;
                }
            }
        }
        fn visit_lit_str(&mut self, literal: &'ast syn::LitStr) {
            if self.1 {
                self.0.push(literal.value());
            }
        }
    }
    let file = syn::parse_file(include_str!("../../domain/src/lib.rs"))?;
    let mut spellings = AsStr(Vec::new(), false);
    spellings.visit_file(&file);
    assert!(
        !spellings.0.is_empty(),
        "ChannelScope::as_str spellings not found"
    );
    Ok(spellings.0)
}

impl<'a> RoleRules<'a> {
    fn check(file: &'a str, modules: &[String], source: &str) -> Result<Self, syn::Error> {
        let mut rules = Self {
            file,
            modules: modules.to_vec(),
            implementation: None,
            self_type: None,
            function: None,
            enclosing: Vec::new(),
            violations: Vec::new(),
            used_comparisons: Vec::new(),
            used_literals: Vec::new(),
            scopes: scope_spellings()?,
        };
        if file.split('/').nth(2) != Some("tests") {
            rules.visit_file(&syn::parse_file(source)?);
        }
        Ok(rules)
    }

    fn identity(&self) -> String {
        let mut parts = vec![self.file.to_owned()];
        parts.extend(self.modules.iter().cloned());
        parts.extend(self.enclosing.iter().cloned());
        if let Some(implementation) = &self.implementation {
            parts.push(format!("impl<{implementation}>"));
        }
        if let Some(function) = &self.function {
            parts.push(function.clone());
        }
        parts.join("::")
    }

    fn refuse(&mut self, form: &'static str) {
        self.violations.push(RoleViolation {
            identity: self.identity(),
            form,
        });
    }

    fn comparison(&mut self, form: &'static str) {
        let identity = self.identity();
        if identity == CAPABILITY_TABLE {
            return;
        }
        if ROLE_COMPARISON_EXCEPTIONS
            .iter()
            .any(|(entry, _)| *entry == identity)
        {
            self.used_comparisons.push(identity);
        } else {
            self.refuse(form);
        }
    }

    fn role_path(&self, path: &syn::Path, qself: Option<&syn::QSelf>) -> bool {
        let mut names: Vec<_> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        if let Some(qself) = qself {
            names.insert(
                0,
                type_name(&qself.ty)
                    .rsplit("::")
                    .next()
                    .unwrap_or("")
                    .to_owned(),
            );
        }
        if names.len() < 2 {
            return false;
        }
        let ty = &names[names.len() - 2];
        let ty = if ty == "Self" {
            self.self_type.as_deref().unwrap_or("")
        } else {
            ty
        };
        is_role_type(ty) && names.last().is_some_and(|name| is_variant_name(name))
    }

    fn pattern_names_role(&self, pattern: &syn::Pat) -> bool {
        struct Paths<'a, 'b> {
            rules: &'a RoleRules<'b>,
            found: bool,
        }
        impl<'ast> Visit<'ast> for Paths<'_, '_> {
            fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
                self.found |= self.rules.role_path(&path.path, path.qself.as_ref());
                visit::visit_expr_path(self, path);
            }
            fn visit_path(&mut self, path: &'ast syn::Path) {
                self.found |= self.rules.role_path(path, None);
                visit::visit_path(self, path);
            }
        }
        let mut paths = Paths {
            rules: self,
            found: false,
        };
        paths.visit_pat(pattern);
        paths.found
    }

    // Any role variant inside an operand counts, not only a bare path: `&role !=
    // &OrganizationRole::Admin`, a cast or a call argument must not hide one.
    fn expression_is_role(&self, expression: &syn::Expr) -> bool {
        struct Paths<'a, 'b> {
            rules: &'a RoleRules<'b>,
            found: bool,
        }
        impl<'ast> Visit<'ast> for Paths<'_, '_> {
            fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
                self.found |= self.rules.role_path(&path.path, path.qself.as_ref());
                visit::visit_expr_path(self, path);
            }
        }
        let mut paths = Paths {
            rules: self,
            found: false,
        };
        paths.visit_expr(expression);
        paths.found
    }

    fn literal(&mut self, value: &str) {
        // Deriving spellings from domain keeps a newly added role or status covered.
        let reserved = domain::OrganizationRole::ALL
            .iter()
            .any(|v| v.as_str() == value)
            || domain::TeamRole::ALL.iter().any(|v| v.as_str() == value)
            || domain::MemberStatus::ALL
                .iter()
                .any(|v| v.as_str() == value)
            || self.scopes.iter().any(|v| v == value);
        if !reserved {
            return;
        }
        let identity = self.identity();
        if identity == SCOPE_SPELLINGS {
            return;
        }
        if ROLE_LITERAL_EXCEPTIONS
            .iter()
            .any(|(entry, literal, _)| *entry == identity && *literal == value)
        {
            self.used_literals.push((identity, value.to_owned()));
        } else {
            self.refuse("literal");
        }
    }

    fn use_tree(&mut self, tree: &syn::UseTree, under_role: bool) {
        match tree {
            syn::UseTree::Path(path) => self.use_tree(
                &path.tree,
                under_role || is_role_type(&path.ident.to_string()),
            ),
            syn::UseTree::Group(group) => {
                for tree in &group.items {
                    self.use_tree(tree, under_role);
                }
            }
            syn::UseTree::Rename(rename) if is_role_type(&rename.ident.to_string()) => {
                self.refuse("use")
            }
            syn::UseTree::Name(name) if name.ident == "self" => {}
            _ if under_role => self.refuse("use"),
            _ => {}
        }
    }

    fn alias(&mut self, ty: &syn::Type) {
        struct RoleTypes<'a> {
            self_type: Option<&'a str>,
            found: bool,
        }
        impl<'ast> Visit<'ast> for RoleTypes<'_> {
            fn visit_path(&mut self, path: &'ast syn::Path) {
                self.found |= path.segments.iter().any(|s| {
                    is_role_type(&s.ident.to_string())
                        || (s.ident == "Self" && self.self_type.is_some_and(is_role_type))
                });
                visit::visit_path(self, path);
            }
        }
        let mut types = RoleTypes {
            self_type: self.self_type.as_deref(),
            found: false,
        };
        types.visit_type(ty);
        if types.found {
            self.refuse("alias");
        }
    }

    // syn deliberately leaves macro bodies opaque. Parse expression arguments where
    // possible; otherwise descend token groups so even json! and macro_rules! literals
    // cannot hide a second spelling. This inspects written source, not macro expansion.
    fn macro_tokens(&mut self, mut cursor: syn::buffer::Cursor<'_>) {
        use syn::parse::Parser;
        if let Ok(file) = syn::parse2::<syn::File>(cursor.token_stream()) {
            self.visit_file(&file);
            return;
        }
        let expressions = Punctuated::<syn::Expr, Token![,]>::parse_terminated;
        if let Ok(expressions) = expressions.parse2(cursor.token_stream()) {
            for expression in expressions {
                self.visit_expr(&expression);
            }
            return;
        }
        // Tokens that are neither items nor expressions (a macro_rules! arm, json!'s
        // object syntax) are not read as syntax. A role type there, other than a method
        // or constant call such as `OrganizationRole::parse`, is refused when the same
        // tokens can compare, match, bind, import or alias: fail closed rather than
        // guess what the expansion does. A variant used only as a value passes.
        if role_tokens(cursor, self.self_type.as_deref()) && binding_tokens(cursor) {
            self.refuse("macro");
        }
        while !cursor.eof() {
            if let Some((_, rest)) = cursor.ident() {
                cursor = rest;
            } else if let Some((inside, _, _, rest)) = cursor.any_group() {
                self.macro_tokens(inside);
                cursor = rest;
            } else if let Some((literal, rest)) = cursor.literal() {
                if let Ok(literal) = syn::parse_str::<syn::LitStr>(&literal.to_string()) {
                    self.literal(&literal.value());
                }
                cursor = rest;
            } else if let Some((_, rest)) = cursor.token_tree() {
                cursor = rest;
            } else {
                break;
            }
        }
    }
}

fn role_tokens(mut cursor: syn::buffer::Cursor<'_>, self_type: Option<&str>) -> bool {
    while !cursor.eof() {
        if let Some((ident, rest)) = cursor.ident() {
            let ident = ident.to_string();
            if (is_role_type(&ident) || (ident == "Self" && self_type.is_some_and(is_role_type)))
                && !role_member_call(rest)
            {
                return true;
            }
            cursor = rest;
        } else if let Some((inside, _, _, rest)) = cursor.any_group() {
            if role_tokens(inside, self_type) {
                return true;
            }
            cursor = rest;
        } else if let Some((_, rest)) = cursor.token_tree() {
            cursor = rest;
        } else {
            break;
        }
    }
    false
}

fn binding_tokens(mut cursor: syn::buffer::Cursor<'_>) -> bool {
    let mut previous = None;
    while !cursor.eof() {
        if let Some((ident, rest)) = cursor.ident() {
            if matches!(
                ident.to_string().as_str(),
                "matches" | "match" | "if" | "let" | "use" | "type"
            ) {
                return true;
            }
            previous = None;
            cursor = rest;
        } else if let Some((punct, rest)) = cursor.punct() {
            let pair = previous.map(|p: char| format!("{p}{}", punct.as_char()));
            if matches!(pair.as_deref(), Some("==" | "!=" | "=>")) {
                return true;
            }
            previous = Some(punct.as_char());
            cursor = rest;
        } else if let Some((inside, _, _, rest)) = cursor.any_group() {
            if binding_tokens(inside) {
                return true;
            }
            previous = None;
            cursor = rest;
        } else if let Some((_, rest)) = cursor.token_tree() {
            previous = None;
            cursor = rest;
        } else {
            break;
        }
    }
    false
}

fn role_member_call(cursor: syn::buffer::Cursor<'_>) -> bool {
    let Some((first, cursor)) = cursor.punct() else {
        return false;
    };
    let Some((second, cursor)) = cursor.punct() else {
        return false;
    };
    first.as_char() == ':'
        && second.as_char() == ':'
        && cursor
            .ident()
            .is_some_and(|(name, _)| !is_variant_name(&name.to_string()))
}

struct MatchesArguments {
    expression: syn::Expr,
    pattern: syn::Pat,
    guard: Option<syn::Expr>,
}

impl syn::parse::Parse for MatchesArguments {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let expression = input.parse()?;
        input.parse::<Token![,]>()?;
        let pattern = syn::Pat::parse_multi_with_leading_vert(input)?;
        let guard = if input.peek(Token![if]) {
            input.parse::<Token![if]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
        Ok(Self {
            expression,
            pattern,
            guard,
        })
    }
}

impl<'ast> Visit<'ast> for RoleRules<'_> {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if let Meta::List(list) = &attribute.meta {
            let buffer = syn::buffer::TokenBuffer::new2(list.tokens.clone());
            self.macro_tokens(buffer.begin());
        } else {
            visit::visit_attribute(self, attribute);
        }
    }

    fn visit_file(&mut self, file: &'ast syn::File) {
        if !is_test_only(&file.attrs) {
            visit::visit_file(self, file);
        }
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        self.enclosing.push(format!("trait<{}>", item.ident));
        visit::visit_item_trait(self, item);
        self.enclosing.pop();
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        let attrs = match item {
            syn::ImplItem::Const(v) => &v.attrs,
            syn::ImplItem::Fn(v) => &v.attrs,
            syn::ImplItem::Type(v) => &v.attrs,
            syn::ImplItem::Macro(v) => &v.attrs,
            _ => return,
        };
        if !is_test_only(attrs) {
            visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        let attrs = match item {
            syn::TraitItem::Const(v) => &v.attrs,
            syn::TraitItem::Fn(v) => &v.attrs,
            syn::TraitItem::Type(v) => &v.attrs,
            syn::TraitItem::Macro(v) => &v.attrs,
            _ => return,
        };
        if !is_test_only(attrs) {
            visit::visit_trait_item(self, item);
        }
    }

    fn visit_item(&mut self, item: &'ast syn::Item) {
        let attrs = match item {
            syn::Item::Const(v) => &v.attrs,
            syn::Item::Enum(v) => &v.attrs,
            syn::Item::ExternCrate(v) => &v.attrs,
            syn::Item::Fn(v) => &v.attrs,
            syn::Item::ForeignMod(v) => &v.attrs,
            syn::Item::Impl(v) => &v.attrs,
            syn::Item::Macro(v) => &v.attrs,
            syn::Item::Mod(v) => &v.attrs,
            syn::Item::Static(v) => &v.attrs,
            syn::Item::Struct(v) => &v.attrs,
            syn::Item::Trait(v) => &v.attrs,
            syn::Item::TraitAlias(v) => &v.attrs,
            syn::Item::Type(v) => &v.attrs,
            syn::Item::Union(v) => &v.attrs,
            syn::Item::Use(v) => &v.attrs,
            _ => return,
        };
        if is_test_only(attrs) {
            return;
        }
        let function = self.function.take();
        let implementation = self.implementation.take();
        let self_type = self.self_type.take();
        let enclosing_len = self.enclosing.len();
        if let Some(implementation) = &implementation {
            self.enclosing.push(format!("impl<{implementation}>"));
        }
        if let Some(function) = &function {
            self.enclosing.push(function.clone());
            self.enclosing.push("<item>".to_owned());
        }
        visit::visit_item(self, item);
        self.enclosing.truncate(enclosing_len);
        self.function = function;
        self.implementation = implementation;
        self.self_type = self_type;
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        self.modules.push(module.ident.to_string());
        visit::visit_item_mod(self, module);
        self.modules.pop();
    }

    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        self.function = Some(function.sig.ident.to_string());
        visit::visit_item_fn(self, function);
        self.function = None;
    }

    fn visit_item_impl(&mut self, implementation: &'ast syn::ItemImpl) {
        let ty = type_name(&implementation.self_ty);
        self.self_type = Some(ty.rsplit("::").next().unwrap_or("").to_owned());
        self.implementation = Some(match &implementation.trait_ {
            Some((path, _)) => format!("{ty} as {}", path_name(path)),
            None => ty,
        });
        visit::visit_item_impl(self, implementation);
        self.implementation = None;
        self.self_type = None;
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        if !is_test_only(&function.attrs) {
            self.function = Some(function.sig.ident.to_string());
            visit::visit_impl_item_fn(self, function);
            self.function = None;
        }
    }

    fn visit_trait_item_fn(&mut self, function: &'ast syn::TraitItemFn) {
        if !is_test_only(&function.attrs) {
            self.function = Some(function.sig.ident.to_string());
            visit::visit_trait_item_fn(self, function);
            self.function = None;
        }
    }

    fn visit_expr_binary(&mut self, expression: &'ast syn::ExprBinary) {
        let form = match expression.op {
            syn::BinOp::Eq(_) => Some("=="),
            syn::BinOp::Ne(_) => Some("!="),
            _ => None,
        };
        if let Some(form) = form
            && (self.expression_is_role(&expression.left)
                || self.expression_is_role(&expression.right))
        {
            self.comparison(form);
        }
        visit::visit_expr_binary(self, expression);
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        if !is_test_only(&arm.attrs) {
            if self.pattern_names_role(&arm.pat) {
                self.comparison("match");
            }
            visit::visit_arm(self, arm);
        }
    }

    fn visit_expr_let(&mut self, expression: &'ast syn::ExprLet) {
        if self.pattern_names_role(&expression.pat) {
            self.comparison("if let");
        }
        visit::visit_expr_let(self, expression);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if !is_test_only(&local.attrs) {
            if local
                .init
                .as_ref()
                .is_some_and(|init| init.diverge.is_some())
                && self.pattern_names_role(&local.pat)
            {
                self.comparison("let else");
            }
            visit::visit_local(self, local);
        }
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.use_tree(&item.tree, false);
        visit::visit_item_use(self, item);
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.alias(&item.ty);
        visit::visit_item_type(self, item);
    }

    fn visit_impl_item_type(&mut self, item: &'ast syn::ImplItemType) {
        if !is_test_only(&item.attrs) {
            self.alias(&item.ty);
            visit::visit_impl_item_type(self, item);
        }
    }

    fn visit_trait_item_type(&mut self, item: &'ast syn::TraitItemType) {
        if !is_test_only(&item.attrs) {
            if let Some((_, ty)) = &item.default {
                self.alias(ty);
            }
            visit::visit_trait_item_type(self, item);
        }
    }

    fn visit_lit_str(&mut self, literal: &'ast syn::LitStr) {
        self.literal(&literal.value());
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if self.file == "crates/domain/src/lib.rs"
            && self.modules.is_empty()
            && self.enclosing.is_empty()
            && self.implementation.is_none()
            && self.function.is_none()
            && mac.path.is_ident("enumeration")
        {
            return;
        }
        if mac
            .path
            .segments
            .last()
            .is_some_and(|s| s.ident == "matches")
        {
            match mac.parse_body::<MatchesArguments>() {
                Ok(arguments) => {
                    if self.pattern_names_role(&arguments.pattern) {
                        self.comparison("matches");
                    }
                    self.visit_expr(&arguments.expression);
                    self.visit_pat(&arguments.pattern);
                    if let Some(guard) = &arguments.guard {
                        self.visit_expr(guard);
                    }
                }
                Err(_) => self.refuse("unparsed matches"),
            }
        } else {
            let buffer = syn::buffer::TokenBuffer::new2(mac.tokens.clone());
            self.macro_tokens(buffer.begin());
        }
    }
}

#[test]
fn production_authorization_uses_capabilities() -> Result<(), Box<dyn Error>> {
    let mut violations = Vec::new();
    let mut used_comparisons = Vec::new();
    let mut used_literals = Vec::new();
    for (file, modules) in ModuleTree::of_workspace()?.production_modules {
        let relative = file
            .strip_prefix(workspace())?
            .to_string_lossy()
            .replace('\\', "/");
        let rules = RoleRules::check(&relative, &modules, &fs::read_to_string(file)?)?;
        violations.extend(rules.violations);
        used_comparisons.extend(rules.used_comparisons);
        used_literals.extend(rules.used_literals);
    }
    assert!(
        violations.is_empty(),
        "role source violations: {violations:#?}"
    );
    let ambiguous = ambiguous_exceptions(
        ROLE_COMPARISON_EXCEPTIONS
            .iter()
            .map(|(entry, _)| *entry)
            .chain(ROLE_LITERAL_EXCEPTIONS.iter().map(|(entry, _, _)| *entry)),
    );
    assert!(
        ambiguous.is_empty(),
        "exception names an ambiguous compound impl: {ambiguous:?}"
    );
    for (entry, reason) in ROLE_COMPARISON_EXCEPTIONS {
        assert!(!reason.is_empty());
        assert!(
            used_comparisons.iter().any(|used| used == entry),
            "stale role exception: {entry}"
        );
    }
    for (entry, literal, reason) in ROLE_LITERAL_EXCEPTIONS {
        assert!(!reason.is_empty());
        assert!(
            used_literals
                .iter()
                .any(|(used, value)| used == entry && value == literal),
            "stale literal exception: {entry}"
        );
    }
    Ok(())
}

// Every generic or compound impl shares one placeholder identity, so an exception
// naming one would cover all of them.
fn ambiguous_exceptions<'a>(entries: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    entries
        .filter(|entry| entry.contains("<compound type>"))
        .collect()
}

#[test]
fn role_rule_bypass_fixtures() -> Result<(), Box<dyn Error>> {
    // A variant added later is recognised by its shape, without editing the rule.
    role_fixture(
        "fn f() { let _ = role == OrganizationRole::Guest; }",
        &["=="],
    )?;
    role_fixture(
        "fn f() { match role { TeamRole::Observer => {} _ => {} } }",
        &["match"],
    )?;
    // Today's variants have the shape the rule recognises.
    for name in domain::OrganizationRole::ALL
        .iter()
        .map(|v| format!("{v:?}"))
        .chain(domain::TeamRole::ALL.iter().map(|v| format!("{v:?}")))
    {
        assert!(is_variant_name(&name), "{name}");
    }
    assert!(!is_variant_name("ALL") && !is_variant_name("parse"));
    // A variant inside an operand counts, not only a bare path.
    for comparison in [
        "&role != &OrganizationRole::Admin",
        "role == *&TeamRole::Manager",
        "role as u8 == OrganizationRole::Owner as u8",
        "Some(role) == Some(TeamRole::Member)",
    ] {
        let form = if comparison.contains("!=") {
            "!="
        } else {
            "=="
        };
        role_fixture(&format!("fn f() {{ let _ = {comparison}; }}"), &[form])?;
    }
    // Unparsed macro bodies fail closed when they can compare, match, bind or alias.
    for source in [
        "macro_rules! check { ($r:expr) => { if $r == OrganizationRole::Admin {} } }",
        "macro_rules! check { ($r:expr) => { matches!($r, TeamRole::Manager) } }",
        "macro_rules! alias { () => { type Chosen = OrganizationRole; } }",
        "macro_rules! import { () => { use domain::TeamRole::*; } }",
    ] {
        let rules = RoleRules::check("crates/app/src/fixture.rs", &[], source)?;
        assert!(
            rules.violations.iter().any(|v| v.form == "macro"),
            "{source}: {:?}",
            rules.violations
        );
    }
    // A variant used only as a value, and member calls, still pass.
    role_fixture(
        r#"fn f() { let _ = json!({"x": OrganizationRole::Member.as_str()}); }"#,
        &[],
    )?;
    role_fixture(
        "macro_rules! parse { ($v:expr) => { OrganizationRole::parse($v) } }",
        &[],
    )?;
    // A compound impl identity cannot carry an exception.
    assert_eq!(
        ambiguous_exceptions(
            [
                "crates/db/src/a.rs::a::impl<<compound type>>::f",
                "crates/db/src/a.rs::a::impl<Event>::f",
            ]
            .into_iter()
        ),
        ["crates/db/src/a.rs::a::impl<<compound type>>::f"]
    );
    // Scope spellings are read from ChannelScope::as_str and match what it returns.
    let mut spellings = scope_spellings()?;
    spellings.sort();
    let mut actual = vec![
        domain::ChannelScope::Organization.as_str().to_owned(),
        domain::ChannelScope::Team(domain::TeamId(uuid::Uuid::nil()))
            .as_str()
            .to_owned(),
    ];
    actual.sort();
    assert_eq!(spellings, actual);
    Ok(())
}

fn role_fixture(source: &str, expected: &[&str]) -> Result<(), Box<dyn Error>> {
    let rules = RoleRules::check("crates/app/src/fixture.rs", &[], source)?;
    let forms: Vec<_> = rules.violations.iter().map(|v| v.form).collect();
    assert_eq!(forms, expected, "{source}");
    Ok(())
}

fn role_variants() -> [(&'static str, &'static str); 5] {
    [
        ("OrganizationRole", "Owner"),
        ("OrganizationRole", "Admin"),
        ("OrganizationRole", "Member"),
        ("TeamRole", "Manager"),
        ("TeamRole", "Member"),
    ]
}

#[test]
fn role_equality_fixtures() -> Result<(), Box<dyn Error>> {
    for (ty, variant) in role_variants() {
        for (operator, form) in [("==", "=="), ("!=", "!=")] {
            for comparison in [
                format!("role {operator} {ty}::{variant}"),
                format!("domain::{ty}::{variant} {operator} role"),
                format!("<domain::{ty}>::{variant} {operator} role"),
                format!("role {operator} ({ty}::{variant})"),
            ] {
                role_fixture(&format!("fn f() {{ if {comparison} {{}} }}"), &[form])?;
            }
            role_fixture(
                &format!(
                    "impl {ty} {{ fn f(self) {{ let _ = self {operator} Self::{variant}; }} }}"
                ),
                &[form],
            )?;
        }
        role_fixture(
            &format!("fn f() {{ let role = {ty}::{variant}; let _ = role.holds(capability); }}"),
            &[],
        )?;
    }
    role_fixture(
        "impl Unrelated { fn f(self) { let _ = self == Self::Admin; } }",
        &[],
    )?;
    Ok(())
}

#[test]
fn role_pattern_fixtures() -> Result<(), Box<dyn Error>> {
    for (ty, variant) in role_variants() {
        for (body, form) in [
            (
                format!("let _ = matches!(role, {ty}::{variant} | {ty}::Member);"),
                "matches",
            ),
            (
                format!("let _ = std::matches!(role, | {ty}::{variant} if ready,);"),
                "matches",
            ),
            (
                format!("match role {{ {ty}::{variant} => true, _ => false }};"),
                "match",
            ),
            (
                format!("if let Some({ty}::{variant}) = role {{}}"),
                "if let",
            ),
            (
                format!("let Some({ty}::{variant}) = role else {{ return; }};"),
                "let else",
            ),
        ] {
            role_fixture(&format!("fn f() {{ {body} }}"), &[form])?;
            role_fixture(
                &format!(
                    "fn f() {{ {} }}",
                    body.replace(&format!("{ty}::"), &format!("<{ty}>::"))
                ),
                &[form],
            )?;
            role_fixture(
                &format!("impl {ty} {{ fn f() {{ {} }} }}", body.replace(ty, "Self")),
                &[form],
            )?;
        }
        role_fixture(
            &format!("fn f() {{ let _ = matches!(role, _ if role == {ty}::{variant}); }}"),
            &["=="],
        )?;
        role_fixture(
            &format!("fn f() {{ assert!(role != {ty}::{variant}); }}"),
            &["!="],
        )?;
        role_fixture(
            &format!("fn f() {{ assert!(matches!(role, {ty}::{variant})); }}"),
            &["matches"],
        )?;
    }
    Ok(())
}

#[test]
fn role_import_and_alias_fixtures() -> Result<(), Box<dyn Error>> {
    for (ty, variant) in role_variants() {
        for import in [
            format!("use domain::{ty}::{variant};"),
            format!("use domain::{ty}::*;"),
            format!("use domain::{{{ty}::{{{variant} as Chosen, *}}}};"),
            format!("use domain::{ty} as Hidden;"),
        ] {
            let count = if import.contains("Chosen") { 2 } else { 1 };
            role_fixture(&import, &vec!["use"; count])?;
        }
        for alias in [
            format!("type Hidden = domain::{ty};"),
            format!("fn f() {{ type Hidden = {ty}; }}"),
            format!("impl Trait for Other {{ type Hidden = {ty}; }}"),
            format!("trait Other {{ type Hidden = {ty}; }}"),
            format!("impl Trait for {ty} {{ type Hidden = Self; }}"),
        ] {
            role_fixture(&alias, &["alias"])?;
        }
        role_fixture(&format!("use domain::{ty}::{{self}};"), &[])?;
        role_fixture(
            &format!("use domain::{{{ty}, Capability}}; type Other = Capability;"),
            &[],
        )?;
    }
    Ok(())
}

#[test]
fn role_literal_fixtures() -> Result<(), Box<dyn Error>> {
    // Independent of the detection's domain-derived set: losing any promised name
    // must fail a fixture rather than shrink both the check and its expectations.
    for value in [
        "owner",
        "admin",
        "member",
        "manager",
        "active",
        "suspended",
        "left",
        "organization",
        "team",
    ] {
        for source in [
            format!("fn f() {{ let _ = role.as_str() == \"{value}\"; }}"),
            format!("const NAME: &str = r#\"{value}\"#;"),
            format!("#[error(\"{value}\")] struct Error;"),
            format!("fn f() {{ let _ = json!({{\"key\": \"{value}\"}}); }}"),
            format!("macro_rules! spelling {{ () => {{ \"{value}\" }} }}"),
        ] {
            role_fixture(&source, &["literal"])?;
        }
    }
    role_fixture(r#"fn f() { let _ = "\x61dmin"; }"#, &["literal"])?;
    role_fixture(
        r#"fn f() { sqlx::query!("SELECT role FROM members WHERE status = 'active'"); let _ = "administrator"; }"#,
        &[],
    )?;
    role_fixture(
        r#"enumeration!(OrganizationRole { Admin => "admin" });"#,
        // Outside domain the unparsed body names a role type beside `=>` as well.
        &["macro", "literal"],
    )?;
    let canonical = RoleRules::check(
        "crates/domain/src/lib.rs",
        &[],
        r#"
        enumeration!(OrganizationRole { Admin => "admin" });
        enumeration!(TeamRole { Manager => "manager" });
        enumeration!(MemberStatus { Active => "active" });
        impl ChannelScope { fn as_str(self) -> &'static str { match self { Self::Organization => "organization", _ => "team" } } }
    "#,
    )?;
    assert!(
        canonical.violations.is_empty(),
        "{:?}",
        canonical.violations
    );
    for source in [
        r#"mod other { enumeration!(TeamRole { Manager => "manager" }); }"#,
        r#"fn f() { enumeration!(OrganizationRole { Admin => "admin" }); }"#,
        r#"impl Other { fn as_str(self) -> &'static str { "team" } }"#,
        r#"impl ChannelScope { fn other(self) -> &'static str { "organization" } }"#,
        r#"impl ChannelScope { fn as_str(self) { fn nested() { let _ = "team"; } } }"#,
    ] {
        let forms: Vec<_> = RoleRules::check("crates/domain/src/lib.rs", &[], source)?
            .violations
            .iter()
            .map(|v| v.form)
            .filter(|form| *form != "macro")
            .collect();
        assert_eq!(forms, ["literal"], "{source}");
    }
    Ok(())
}

#[test]
fn role_capability_table_fixture() -> Result<(), Box<dyn Error>> {
    for (ty, variant) in role_variants() {
        let body =
            format!("let _ = matches!(role, {ty}::{variant}); let _ = || role == {ty}::{variant};");
        let source = format!("impl OrganizationRole {{ fn holds() {{ {body} }} }}");
        assert!(
            RoleRules::check("crates/domain/src/lib.rs", &[], &source)?
                .violations
                .is_empty()
        );
        for (file, modules, source) in [
            ("crates/domain/src/other.rs", vec![], source.clone()),
            (
                "crates/domain/src/lib.rs",
                vec!["other".to_owned()],
                source.clone(),
            ),
            (
                "crates/domain/src/lib.rs",
                vec![],
                format!("mod other {{ {source} }}"),
            ),
            (
                "crates/domain/src/lib.rs",
                vec![],
                format!("impl TeamRole {{ fn holds() {{ {body} }} }}"),
            ),
            (
                "crates/domain/src/lib.rs",
                vec![],
                format!("impl Other for OrganizationRole {{ fn holds() {{ {body} }} }}"),
            ),
            (
                "crates/domain/src/lib.rs",
                vec![],
                format!("fn holds() {{ {body} }}"),
            ),
        ] {
            assert_eq!(
                RoleRules::check(file, &modules, &source)?.violations.len(),
                2,
                "{file}: {source}"
            );
        }
        let source =
            format!("impl OrganizationRole {{ fn holds() {{ fn nested() {{ {body} }} }} }}");
        assert_eq!(
            RoleRules::check("crates/domain/src/lib.rs", &[], &source)?
                .violations
                .len(),
            2
        );
    }
    Ok(())
}

#[test]
fn role_function_exception_fixture() -> Result<(), Box<dyn Error>> {
    let file = "crates/db/src/operations/membership.rs";
    let modules = vec!["operations".to_owned(), "membership".to_owned()];
    for (ty, variant) in role_variants() {
        let body = format!("let _ = role == {ty}::{variant}; let _ = || role != {ty}::{variant};");
        let source = format!("fn change_member_role() {{ {body} }}");
        assert!(
            RoleRules::check(file, &modules, &source)?
                .violations
                .is_empty()
        );
        for (file, modules, source) in [
            (
                "crates/app/src/membership.rs",
                modules.clone(),
                source.clone(),
            ),
            (
                file,
                vec!["operations".to_owned(), "elsewhere".to_owned()],
                source.clone(),
            ),
            (
                file,
                modules.clone(),
                format!("mod elsewhere {{ {source} }}"),
            ),
            (file, modules.clone(), format!("impl Other {{ {source} }}")),
            (
                file,
                modules.clone(),
                format!("fn change_member_role() {{ fn nested() {{ {body} }} }}"),
            ),
            (
                file,
                modules.clone(),
                format!("fn change_member_role() {{ const NESTED: fn() = || {{ {body} }}; }}"),
            ),
        ] {
            assert_eq!(
                RoleRules::check(file, &modules, &source)?.violations.len(),
                2,
                "{file}: {source}"
            );
        }
        assert_eq!(
            RoleRules::check(file, &modules, &format!("trait Other {{ {source} }}"))?
                .violations
                .len(),
            2
        );
        let source = format!(
            "fn change_member_role() {{ use domain::{ty}::*; type Hidden = {ty}; let _ = \"admin\"; }}"
        );
        assert_eq!(
            RoleRules::check(file, &modules, &source)?.violations.len(),
            3
        );
    }
    Ok(())
}

#[test]
fn role_literal_exception_fixture() -> Result<(), Box<dyn Error>> {
    let file = "crates/db/src/audit.rs";
    let modules = vec!["audit".to_owned()];
    for (function, value) in [
        ("invite_created", "organization"),
        ("invite_revoked", "organization"),
        ("organization_created", "organization"),
        ("team_created", "team"),
    ] {
        let method = format!("fn {function}() {{ let _ = \"{value}\"; }}");
        assert!(
            RoleRules::check(file, &modules, &format!("impl Event {{ {method} }}"))?
                .violations
                .is_empty()
        );
        for source in [
            method.clone(),
            format!("impl Event<Other> {{ {method} }}"),
            format!("impl Other {{ {method} }}"),
            format!("impl Other for Event {{ {method} }}"),
            format!("mod elsewhere {{ impl Event {{ {method} }} }}"),
            format!(
                "impl Event {{ fn {function}() {{ fn nested() {{ let _ = \"{value}\"; }} }} }}"
            ),
            format!("impl Event {{ fn {function}() {{ let _ = \"admin\"; }} }}"),
            format!(
                "impl Event {{ fn {function}() {{ let _ = role == OrganizationRole::Admin; }} }}"
            ),
        ] {
            assert_eq!(
                RoleRules::check(file, &modules, &source)?.violations.len(),
                1,
                "{source}"
            );
        }
        assert_eq!(
            RoleRules::check(
                "crates/app/src/audit.rs",
                &modules,
                &format!("impl Event {{ {method} }}")
            )?
            .violations
            .len(),
            1
        );
    }
    Ok(())
}

#[test]
fn role_test_code_fixture() -> Result<(), Box<dyn Error>> {
    role_fixture(r#"#![cfg(test)] const VALUE: &str = "admin";"#, &[])?;
    role_fixture(
        r#"impl Other { #[cfg(test)] const VALUE: &str = "admin"; }"#,
        &[],
    )?;
    role_fixture(
        r#"trait Other { #[cfg(test)] const VALUE: &str = "manager"; }"#,
        &[],
    )?;
    for (ty, variant) in role_variants() {
        let source = format!(
            r#"
            use domain::{ty}::*;
            type Hidden = {ty};
            fn f() {{
                let _ = role == {ty}::{variant};
                let _ = role != {ty}::{variant};
                let _ = matches!(role, {ty}::{variant});
                match role {{ {ty}::{variant} => (), _ => () }};
                if let {ty}::{variant} = role {{}}
                let {ty}::{variant} = role else {{ return; }};
                let _ = "admin";
            }}
        "#
        );
        role_fixture(&format!("#[cfg(test)] mod checks {{ {source} }}"), &[])?;
        role_fixture(&format!("#[cfg(test,)] mod checks {{ {source} }}"), &[])?;
        assert!(
            RoleRules::check("crates/app/tests/fixture.rs", &[], &source)?
                .violations
                .is_empty()
        );
        assert_eq!(
            RoleRules::check("crates/app/src/tests/fixture.rs", &[], &source)?
                .violations
                .len(),
            9
        );
        assert_eq!(
            RoleRules::check("crates/app/src/fixture_tests.rs", &[], &source)?
                .violations
                .len(),
            9
        );
        assert_eq!(
            RoleRules::check(
                "crates/app/src/fixture.rs",
                &[],
                &format!("#[cfg(not(test))] mod live {{ {source} }}")
            )?
            .violations
            .len(),
            9
        );
        role_fixture(
            &format!("#[cfg(test)] fn f() {{ let _ = role == {ty}::{variant}; }}"),
            &[],
        )?;
        role_fixture(
            &format!("impl {ty} {{ #[cfg(test)] fn f() {{ let _ = self == Self::{variant}; }} }}"),
            &[],
        )?;
    }
    Ok(())
}
