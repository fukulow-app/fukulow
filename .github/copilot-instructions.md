# Review instructions

These checks are an extract of the documents under `docs/` —
`architecture/invariants.md`, `security/authorization-model.md`,
`architecture/overview.md` — and of `AGENTS.md`. **Those are the source.** If a
check here disagrees with them, they win, and this file is wrong. A rule that
appears here and nowhere there should not be here.

Each check is phrased so the answer is yes or no. Comment only where the answer
is no, and say which check.

## Request handling

- Is there an `unwrap()`, `expect()` or `panic!` on a path that handles a request?
  `expect()` is allowed only at startup, where failing to start is correct, and its
  message must name what is missing — **never a value**.

## Logs and errors

- Could a log line or error message contain a **message body, token, password,
  email address or invite link**? This includes values interpolated into errors
  that are later logged.

## Tenant isolation

- Does every query that reads or writes organization data filter by
  `organization_id`?
- Is SQL written anywhere outside the `db` crate?

## Authorization

- Does the route decide access with exactly one of `is_organization_member` or
  `can_access_channel`? Flag a route that uses neither.
- Are **a channel's contents** guarded by organization membership? That locks out
  members from outside the organization, who are allowed.
- Are **an organization's resources** guarded by channel membership? That exposes
  them to another organization.
- Does code compare a **role name** (`role == ...`, `matches!(role, ...)`) to
  decide access? It should ask for a capability instead.
- Does a resource that belongs to an organization or channel the caller cannot see
  return anything other than **404**? A 403 confirms it exists.

## Messages

- Is a **message id** used to order messages or as a cursor? Ordering uses
  `channel_seq`.

## Interface text

- Is text shown to a user **hard-coded in source** — Rust or TypeScript? It goes
  through the translation file.

## Tests

- Would a new test fail if the change it tests were reverted? Flag a test that only
  exercises the path where the guard is not needed.
- Does a test that claims to cover concurrency actually run its operations
  concurrently?

## Changes to how the project is reviewed or built

- Does the pull request change `.github/copilot-instructions.md`,
  `.github/instructions/`, `.github/workflows/`, `.github/CODEOWNERS`, `AGENTS.md`
  or `deny.toml`? Say so at the top of the review. **These instructions are read
  from the pull request's own branch**, so a change here changes this review.
