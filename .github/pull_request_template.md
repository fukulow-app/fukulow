Closes #

## What changes

<!-- One or two sentences. What the reader will see differently after this. -->

## Why

<!-- What could not be done before. If a dependency was added, say what it does
     that could not be done without it, and what alternatives were considered. -->

## How it was verified

<!-- Commands, and what they printed. Not "it works".
     For a new test: state that it was run against the unfixed code and failed. -->

```
```

## Checklist

- [ ] Commits are signed off (`git commit -s`)
- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --all`
- [ ] No message bodies, tokens, email addresses or invite links reach the logs
- [ ] Japanese strings go through the translation file, not into the code
- [ ] **This changes an invariant** — if checked, `docs/architecture/invariants.md`
      is updated in this pull request, and says how the new rule is enforced
- [ ] **This touches something that would need a full data migration to change**
      (identifier scheme, naming, tenant isolation, cursor meaning, frame shape)
      — if checked, say which, and confirm the issue says so too
