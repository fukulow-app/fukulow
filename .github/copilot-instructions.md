# Review instructions

These checks are an extract of the documents under `docs/` —
`architecture/invariants.md`, `security/authorization-model.md`,
`architecture/overview.md` — and of `AGENTS.md`. **Those are the source.** If a
check here disagrees with them, they win, and this file is wrong. A rule that
appears here and nowhere there should not be here.

**Each item below is a condition to flag.** Comment when the change meets it, and
say which item. Do not comment when it does not.

## Request handling

- **Flag** `unwrap()`, `expect()` or `panic!` on a path that handles a request.
  `expect()` at startup is not flagged when failing to start is correct and its
  message names what is missing — **but flag it if the message includes a value**.

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
  `channel_members` (cross-tenant), `actors` and `users` (global) have no such
  column and are reached through a channel or a membership; do not flag those for
  its absence.
- **Flag** authorship, membership or an audit record that references `users` rather
  than `actors`. Every operation is performed by an actor; `users` holds only a
  person's email address and password.
- **Flag** application SQL outside the `db` crate. SQL under `migrations/` is not
  flagged.

## Authorization

- **Flag** a route that reads or changes a protected resource without using
  `is_organization_member` or `can_access_channel`. Using both is allowed.
  Operational routes such as `/health` are not flagged.
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

## Changes to how the project is reviewed or built

- **Flag, at the top of the review,** any change to
  `.github/copilot-instructions.md`, `.github/instructions/`,
  `.github/workflows/`, `.github/CODEOWNERS`, `AGENTS.md` or `deny.toml`. These
  instructions are read from the pull request's own branch, so a change to them
  changes this review.
