---
applyTo: "migrations/**"
---

# Migration review

The schema's rules are in `docs/architecture/invariants.md`. A migration is
expensive to reverse once a self-hoster has run it, so check each of these.

## Every new table

- Does the pull request say which class the table is in — tenant-owned,
  cross-tenant, global or internal?
- Is there a matching `.down.sql`?
- Are all timestamps `timestamptz`? Flag `timestamp` without a time zone.

## Tenant-owned tables

- Is there `organization_id NOT NULL`?
- Does a parent table have `UNIQUE (id, organization_id)`, and does its child
  reference it with a **composite** foreign key `(parent_id, organization_id)`? A
  plain foreign key on `parent_id` alone lets the child claim another organization.
- If a column in a composite foreign key is nullable, is there a comment saying the
  key is **not checked** when that column is NULL? It looks enforced otherwise.
- Are row security `ENABLE` **and** `FORCE` both present, once row security exists?

## Privileges

- Is there `ALTER DEFAULT PRIVILEGES`? Flag it. Privileges are granted table by
  table, because a missing `GRANT` fails loudly and a missing `REVOKE` never does.
- Is any role created or altered with `BYPASSRLS`? Flag it — this is never allowed.
- Is `UPDATE` or `DELETE` on `audit_events` granted to `fukulow_app`?
- Is `DELETE` on `users` or `organization_members` granted to `fukulow_app`? Those
  rows are never deleted.

## Columns

- Does any new column store a **copy of a display name**? Names are always resolved
  to the current one.
- Is a default given to a column whose meaning a default would guess —
  `invites.kind`, `channels.scope`, `audit_events.actor_kind`?
- Is an email looked up or indexed without `lower(...)`?
