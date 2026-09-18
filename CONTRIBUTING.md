# Contributing

Thanks for looking. This is a hobby project run by one person, so a few things
are stricter than you might expect — they exist to keep it maintainable by one
person, and safe to run in public.

## Before writing code

**Open an issue first.** A pull request that arrives without one is likely to
conflict with something already decided but not yet written down.

## Sign your commits off (DCO)

Every commit must carry a `Signed-off-by` line certifying the
[Developer Certificate of Origin](https://developercertificate.org/):

```bash
git commit -s -m "your message"
```

There is no CLA. The sign-off is the whole agreement.

## Ground rules

- **Work on a branch, never on `main`.** Run
  `git config core.hooksPath .githooks` after cloning; the hooks refuse commits
  and pushes on `main`, and scan staged changes for secrets.
- **English** in code, comments, documentation, commit messages, issues and
  pull requests. Japanese belongs in the translation files only.
- **Do not add a dependency without saying why in the pull request** — what
  could not be done without it, and what alternatives were considered.
- **No `unsafe`.** No `unwrap()`, `expect()`, `panic!`, `dbg!` or printing to
  stdout or stderr in production code. Workspace clippy lints deny these outside
  tests in the `build` job. A justified exception uses
  `#[expect(clippy::…, reason = "…")]` at the site, never `#[allow]`;
  `expect()` is only for cases where failing to start is correct, naming what is
  missing and never a value.
- **Never log message bodies, tokens, email addresses or invite links.**
- **Do not copy code from other chat projects.** Several are under licenses
  incompatible with this one. Learning from their design is fine; pasting their
  code is not.

Clippy's `allow-*-in-tests` options exempt test functions and `#[cfg(test)]`
modules, but do not reach ordinary integration-test helpers. Each `tests/*.rs`
therefore allows only the six production-call lints at crate level, with a reason.
`allow_attributes` rejects outer `#[allow]` attributes but ignores inner
`#![allow]` attributes; `allow_attributes_without_reason` still checks both.
So `crates/app/tests/source_rules.rs` refuses any inner lint attribute in production
code, and requires every production `expect(clippy::…)` to be listed in
`PRODUCTION_CLIPPY_EXPECTATIONS` there: adding an exception means editing that list,
where its reason is read. See
[`docs/architecture/invariants.md`](docs/architecture/invariants.md).

## Adding an HTTP route

Add every route to `crates/app/src/route_registry.rs`, with its method, path,
WebSocket upgrade flag and session requirement. State changes are derived from
POST, PUT, PATCH, DELETE or an upgrade; they are not a separate flag. The router
and the table-driven HTTP contract sweep both use this registry. A source test
rejects route registration elsewhere in `crates/app/src` (inline `#[cfg(test)]`
modules are excluded).

The sweep checks Origin before credentials, missing and unknown sessions, empty
errors without Set-Cookie, and the `/api/v1` prefix. It also checks that a route
marked public does not require a session. The route's own tests still cover its
body, capability, and a resource in another organization returning 404; keep
existing order and error tests that exercise those details.

## What a pull request needs

A new invariant in [`docs/architecture/invariants.md`](docs/architecture/invariants.md)
comes with its enforcement mechanism and a test shown failing when that mechanism
is removed. If enforcement still depends on people, use **Review** in its
*Enforced by* cell and cite the issue that will replace it with a mechanism.
`crates/app/tests/invariants_document.rs` requires that citation whenever the cell
contains the word "review", in any case.

- A linked issue
- A description of what changes and why
- Tests that fail without the change — **check that they fail**, a test that
  passes against the unfixed code is not evidence of anything
- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test --all` passing
