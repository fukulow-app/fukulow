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
- **No `unsafe`.** No `unwrap()`, `expect()` or `panic!` on a request path.
- **Never log message bodies, tokens, email addresses or invite links.**
- **Do not copy code from other chat projects.** Several are under licenses
  incompatible with this one. Learning from their design is fine; pasting their
  code is not.

## What a pull request needs

- A linked issue
- A description of what changes and why
- Tests that fail without the change — **check that they fail**, a test that
  passes against the unfixed code is not evidence of anything
- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test --all` passing
