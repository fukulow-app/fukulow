# Review instructions

These checks are an extract of the documents under `docs/` —
`architecture/invariants.md`, `security/authorization-model.md`,
`architecture/overview.md` — and of `AGENTS.md`. **Those are the source.** If a
check here disagrees with them, they win, and this file is wrong. A rule that
appears here and nowhere there should not be here.

**Each item below is a condition to flag.** Comment when the change meets it, and
say which item. Do not comment when it does not.

## Production code

- **Flag** `unwrap()`, `expect()`, `panic!`, `dbg!` or printing to stdout or stderr
  in production code without a justified `#[expect(clippy::…, reason = "…")]`
  at the site. Workspace clippy lints deny these outside tests in the `build` job.
  `expect()` at startup is only justified when failing to start is correct and its
  message names what is missing — **flag it if the message includes a value**.
- **Flag** a new entry in `PRODUCTION_CLIPPY_EXPECTATIONS`
  (`crates/app/tests/source_rules.rs`) whose reason is not that failing to start is
  correct. The test makes every production exception an edit to that list; whether
  the reason holds is the part left to judgement.

## Logs and errors

- **Flag** a log line or error message that could contain a **message body, token,
  password, email address or invite link**, including values interpolated into an
  error that is later logged.

## Personal data in stored metadata

- **Flag** a display name, email address or other personal data written into
  `jsonb` or other structured metadata — audit metadata in particular. Display
  names live only in `actors` and `organization_members`. Audit records can never be
  updated or deleted, so personal data written there could never be erased.
- **Flag** an audit metadata constructor that accepts free text (`String`, `&str`,
  `serde_json::Value`) rather than ids and enumerations.

## Tenant isolation

- **Flag** a read **or write** of tenant-owned data — `SELECT`, `INSERT`,
  `UPDATE` or `DELETE` — that is not scoped by `organization_id`. An `INSERT`
  that does not set it, or an `UPDATE`/`DELETE` that does not filter by it, is as
  much a violation as a `SELECT`.
  `channel_members` (cross-tenant), `actors`, `users` and `sessions` (global) have no
  such column and are reached through a channel or a membership; do not flag those for
  its absence. Do not flag `leave_service` reading the acting actor's own
  `organization_members` and `channel_members` rows by `actor_id` alone: it is the one
  service-wide exception in `AGENTS.md`, and its writes are scoped by the
  `organization_id` those rows name. The second exception is invite acceptance's
  pre-check and conditional use by globally unique token hash, before an organization
  is known; every following write is scoped by the returned `organization_id`.
- **Flag** authorship, membership or an audit record that references `users` rather
  than `actors`. Every operation is performed by an actor; `users` holds a person's
  personal data (email address, password) keyed by their actor.
- **Flag** application SQL outside the `db` crate. SQL under `migrations/` is not
  flagged.

## Authorization

- **Flag** a route that reads or changes a protected resource without using
  `is_organization_member`, `can_access_channel`, or an operation using `authorize`
  (which checks active organization membership before its capability table).
  Combining them is allowed.
  Operational routes such as `/health` are not flagged.
- **Flag** a route that checks authorization in its own transaction before calling
  a state-changing operation. The operation must decide its capability in the
  writing transaction from membership locked `FOR SHARE`, after actor and
  organization locks, in that order.
- **Flag** the contents of a **team-scoped** channel guarded by organization
  membership. That locks out members from outside the organization, whom the model
  allows. (An organization-scoped channel *is* guarded by organization membership;
  do not flag that.)
- **Flag** an organization's own resources — its teams, members, invites,
  settings — guarded by channel membership. That exposes them to another
  organization.
- **Flag** access decided by comparing a **role name** (`role == ...`,
  `matches!(role, ...)`). Access is decided by asking for a capability.
- **Flag** a response other than **404** for a resource belonging to an
  organization or channel the caller cannot see. A 403 confirms it exists.

## Messages

- **Flag** a message id used to order messages or as a cursor. Ordering uses
  `channel_seq`.

## Interface text

- **Flag** text shown to a user hard-coded in source — Rust or TypeScript. It goes
  through the translation file.

## Tests

- **Flag** a new or changed test that would still pass if the change it covers
  were reverted. It is not evidence of the change.
- **Flag** a test that claims to cover concurrency but runs its operations one
  after another.

## Scope of a change

- **Flag** reformatting, renaming or restructuring of code the linked issue does not
  ask for, and a pull request that both changes behaviour and refactors. Name the
  files or hunks that fall outside the issue.
- **Flag** a new trait, generic parameter or layer that has a single implementation
  — a test double counts as one — when no issue calls for another.

## Changes to how the project is reviewed or built

- **Flag, at the top of the review,** any change to
  `.github/copilot-instructions.md`, `.github/instructions/`,
  `.github/workflows/`, `.github/CODEOWNERS`, `AGENTS.md` or `deny.toml`. These
  instructions are read from the pull request's own branch, so a change to them
  changes this review.
