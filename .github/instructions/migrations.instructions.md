---
applyTo: "migrations/**"
---

# Migration review

The schema's rules are in `docs/architecture/invariants.md`; table classes are
defined in `docs/domain/glossary.md`. A migration is expensive to reverse once a
self-hoster has run it.

**Each item below is a condition to flag.** Comment when the migration meets it.

## Every new table

- **Flag** a new table whose pull request does not say which class it is in:
  tenant-owned, cross-tenant, global or internal.
- **Flag** an up migration with no matching `.down.sql`.
- **Flag** a `timestamp` column without a time zone. Timestamps are `timestamptz`.

## Tenant-owned tables

- **Flag** a tenant-owned table, other than `organizations` itself, without
  `organization_id NOT NULL`.
- **Flag** a foreign key from a tenant-owned table to a tenant-owned parent that
  **does not include `organization_id`**, whatever the other column is called —
  `team_id`, `channel_id`, an invite target. The reference is composite,
  `(<parent key>, organization_id)`, against a parent with
  `UNIQUE (id, organization_id)`; otherwise the child can claim another
  organization. **`channel_members` is exempt**: it is cross-tenant by design and
  must not carry this constraint.
- **Flag** a composite foreign key with a nullable column and no comment saying the
  key is **not checked** when that column is NULL. It looks enforced otherwise.
- **Flag** a new tenant-owned or cross-tenant table without both `ENABLE` and
  `FORCE ROW LEVEL SECURITY` — **if any existing migration already enables row
  security**. Before row security is introduced, tables have none by design.

## Privileges

- **Flag** `ALTER DEFAULT PRIVILEGES`. Privileges are granted table by table: a
  missing `GRANT` fails loudly, a missing `REVOKE` never does.
- **Flag** any role created or altered with `BYPASSRLS`. Never allowed.
- **Flag** `UPDATE` or `DELETE` on `audit_events` granted to `fukulow_app`.
- **Flag** `DELETE` on `actors` or `organization_members` granted to `fukulow_app`.
  Those rows are never deleted. (`DELETE` on `users` is expected: a person's `users`
  row is deleted when they leave the service.)

## Columns

- **Flag** a column that stores a display name in any table other than `actors` or
  `organization_members`.
- **Flag** a foreign key for membership, authorship or audit that references
  `users` instead of `actors`, and an `organization_id` on `actors` — actors are
  global.
- **Flag** a default on `invites.kind`, `channels.scope`, `actors.type` or
  `audit_events.actor_id`. A default would guess their meaning — and a default
  actor would attribute an audit record to someone who did not do it.
- **Flag** an email column indexed or looked up without `lower(email COLLATE "C")`.
  Without the collation, case folding depends on the server's locale.
