# AGENTS.md

For AI coding tools and the people using them. Short on purpose: it points at
where the rules are, and repeats only the few that are dangerous to miss.

## Read before changing anything

| | |
|---|---|
| [`docs/architecture/invariants.md`](docs/architecture/invariants.md) | Rules that must always hold, and how each is enforced |
| [`docs/security/authorization-model.md`](docs/security/authorization-model.md) | Who may see and do what |
| [`docs/domain/glossary.md`](docs/domain/glossary.md) | What words mean. Use these identifiers |
| [`docs/architecture/overview.md`](docs/architecture/overview.md) | Crates and which way they may depend |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | Branches, sign-off, what a pull request needs |

## Never

These are repeated here deliberately. Each is cheap to state and expensive to get
wrong.

- **Never log a message body, token, password, email address or invite link.**
  Not in errors either
- **Never read or write tenant-owned data without filtering by `organization_id`.**
  `organizations` itself is the root and is scoped by its own `id`.
  `channel_members` (cross-tenant) is reached through its channel. `actors`, `users`
  and `sessions` are global: in a tenant context reach them through a membership;
  signing in and other identity lookups need none. Never write application SQL outside the `db` crate —
  migrations are the exception
- **Never point authorship, membership or audit at `users`.** They reference
  `actors`. `users` holds a person's personal data — email address and password —
  and no other personal data lives anywhere else
- **Never decide access by role name.** Ask for a capability
- **Never use `SET` for the row-security actor (`fukulow.actor_id`); use
  `SET LOCAL`** inside the transaction
- **Never create a role with `BYPASSRLS`, and never use
  `ALTER DEFAULT PRIVILEGES`**
- **Never `UPDATE` or `DELETE` an audit record**
- **Never use a message id to order messages.** Use `channel_seq`
- Never `unwrap()`, `expect()` or `panic!` on a request path. Never `unsafe`
- Never hard-code text shown on screen
- **Never add a dependency without saying why** in the pull request — what could
  not be done without it

## Before saying something works

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
```

**A new test must fail against the code without the change.** Run it that way
once. A test that passes either way is not evidence.

**A concurrency test must run genuinely concurrently.** Sequential execution
cannot exercise the interleaving it exists to catch.

## Writing

- Code, comments, documentation, commits, issues and pull requests are **English**
- **Comments say why, not what.** A rejected alternative, a trap, the reason an
  order matters, something deliberately not done. Not a restatement of the code
- Work on a branch named `issue-<number>-<short-slug>`, never on `main`
- Sign off every commit: `git commit -s`
