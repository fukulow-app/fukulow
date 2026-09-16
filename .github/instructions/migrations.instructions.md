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
- **Flag** a child that references a tenant-owned parent by `parent_id` alone.
  The reference is a **composite** foreign key `(parent_id, organization_id)`
  against a parent with `UNIQUE (id, organization_id)` — otherwise the child can
  claim another organization.
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
- **Flag** `DELETE` on `users` or `organization_members` granted to `fukulow_app`.
  Those rows are never deleted.

## Columns

- **Flag** a column that stores a display name in any table other than `users` or
  `organization_members`.
- **Flag** a default on `invites.kind`, `channels.scope` or
  `audit_events.actor_kind`. A default would guess their meaning.
- **Flag** an email column indexed or looked up without `lower(...)`.
