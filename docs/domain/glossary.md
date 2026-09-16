# Glossary

The words used in code, the API, the database and these documents — and what the
person using fukulow sees instead.

**Identifiers never change. Labels can.** The interface is Japanese first, and its
wording lives in one translation file. A label can be reworded in that one file;
renaming an identifier means changing every table, route and type that uses it.
If a label seems wrong, change the label.

## The structure

| Identifier | Meaning | Label (ja) |
|---|---|---|
| `organization` | A company or other body. **The tenant**: every piece of data belongs to exactly one, except people | 組織 |
| `team` | A grouping inside an organization — a store, a department, a project. **Describes where someone belongs, not who receives what** | チーム |
| `channel` | A place where a conversation happens. Owned by a team or directly by the organization | ルーム |
| `direct` | A one-to-one or small-group conversation | ダイレクト |
| `member` | A person's membership of an organization, a team or a channel | メンバー |

**Not used anywhere**, in identifiers or labels: `workspace`, `server`, `guild`.
`server` collides with the machine; `workspace` means nothing to most people who
will use this.

## People

| Term | Meaning |
|---|---|
| **user** | A person. **Global**: a user is not owned by any organization, and one user can belong to several. Survives leaving every organization |
| **organization member** | A user's membership of one organization. Carries the role, the status and an optional display name for that organization |
| **status** | `active`, `suspended` or `left`. **A security rule, not a label** — row security only admits `active` members |
| **display name** | Always the current one. Past messages show the name as it is now, not as it was when they were written |
| **leaving the service** | The user row stays; personal data in it is erased and `deleted_at` is set. Deleting the row would take the author of every past message with it |
| **actor** | Whoever performs an action — a user today, possibly a service account later. Authorization asks about an actor, not a user, so that does not change shape later |

## Channels

| Term | Meaning |
|---|---|
| **scope** | Which container owns a channel: `organization` or `team`. An organization-scoped channel belongs to no team — a company-wide notice lives there |
| **implicit membership** | Every active member of an organization can see its organization-scoped channels **without being listed in them**. Joining the organization is joining those channels |
| **cross-organization member** | A user in a channel who is not a member of the channel's organization. Allowed by design, and the reason channel access is not decided by organization membership |
| **`channel_seq`** | A message's position in its channel, assigned by the server. **The ordering key and the cursor.** A message's `id` is not |
| **message id** | Chosen by the client (UUIDv7), so a message can be drawn before it is stored and a retry does not create a duplicate. **Identity only — never used to order** |

## Authorization

| Term | Meaning |
|---|---|
| **tenant** | An organization. Tenant isolation means one organization's data is never visible to another |
| **membership** | Whether someone belongs to an organization, team or channel |
| **role** | Where someone stands within an organization or team: `owner` / `admin` / `member`, or `manager` / `member` |
| **capability** | Something that can be done. **Code asks for a capability, never for a role name** |
| **audience** | Who receives something. **Separate from who may send it** |
| **access primitives** | `is_organization_member` and `can_access_channel`. The only two ways to ask whether someone may see something |

## Invites

| Term | Meaning |
|---|---|
| **invite kind** | What an invite admits to: `organization` or `team` now; `channel` and `connection` later. No default |
| **use count** | How many times an invite has been accepted. Incremented in the same statement that checks the limit |

## Data classes

Every table is in exactly one class, and a new table says which.

| Class | Meaning | Row security |
|---|---|---|
| **tenant-owned** | Belongs to one organization; carries `organization_id`. **`organizations` is the root: it is the tenant, and its own `id` is the value others carry** | Policy derived from membership |
| **cross-tenant** | Links an organization's resource to users who may be outside it | Policy, not a simple comparison |
| **global** | Belongs to no organization | Cannot be scoped by organization |
| **internal** | Bookkeeping, such as the migration table | None; the application has no access |
