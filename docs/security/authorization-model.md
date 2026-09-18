# Authorization model

Who may see and do what, and where that is decided.

**This is the model, not a permission list.** The set of capabilities is built up
as features land; a table of them written ahead of the features would describe
things that do not exist. What is fixed here is the grammar those capabilities
are written in.

## The tenant is the organization

Every tenant-owned piece of data belongs to exactly one organization.
**`organizations` is the root** — each row is a tenant. **Actors are global** (see
below): one person, bot or integration can belong to several organizations and
keeps the same identity across them. Bookkeeping, such as the migration table,
belongs to no organization.

## Who acts

**Every operation is performed by an actor** — a `human`, a `bot`, an `integration`
or the `system`. Memberships, message authorship and audit records all reference
the actor, so one set of rules covers all of them: a bot added to an organization is
isolated exactly as a person is.

A person's email address and password are not on the actor; they are in `users`,
which belongs to a human actor. Only a person can hold a login session.

**REST, webhooks and WebSocket are ways in, not actors.** An integration that
reaches Fukulow all three ways is one actor. Every credential — a session, an API
key, a token — resolves to an actor.

**Something becomes an actor when what it does must be recorded as done by it** —
posting, reacting, changing a channel. A service Fukulow only reads from is not an
actor.

**One organization's data is never visible to another** — with one deliberate,
explicit exception: channel members from outside the organization (below).

## Scopes

```
organization
├── team
│    └── channel (scope = team)
└── channel      (scope = organization)
```

A channel is owned **either** by a team **or** directly by its organization. An
organization-scoped channel belongs to no team, which is what lets a notice from
head office reach someone who is only in one store's team.

## Four separate things

| | Question it answers | Example |
|---|---|---|
| **Membership** | Does this person belong here? | A is in the Shibuya store team |
| **Role** | Where do they stand here? | B is an `admin` of the organization |
| **Capability** | May they do this? | May B post a company-wide notice? |
| **Audience** | Who receives it? | Every active member of the organization |

**Keep them apart.** Where someone belongs does not decide what they may manage:
B can post to every store without being a member of any store's team. Who may send
a notice does not decide who receives it.

## Ask for a capability, never a role

```rust
// No
if member.role == Role::Admin { ... }

// Yes
match authorize(conn, actor, organization, Capability::InviteMember).await? { ... }
```

`Capability::InviteMember` is held by owners and admins. It authorizes both creating
and revoking organization invites; members do not hold it. State-changing operations
must decide their capability inside the transaction that writes, from an active
membership row locked `FOR SHARE`. The crate-private `db` helper
`authorize(conn, actor, organization, capability)` asks `OrganizationRole::holds`
in `domain` and returns `Allowed`, `Forbidden`, or `NotAMember`. Invite creation
and revocation use it; routes map the operation's refusal to 403 or 404 and do
not run a separate authorization transaction. A suspended owner is not an active
member and receives 404. An active member without the capability receives 403.

The lock order is **actor → organization → membership**: call `lock_actor` and
`lock_organization` before `authorize`. A missing or invisible organization also
returns `NotAMember`. Taking the membership first can deadlock with a demotion:
the demotion holds the organization `FOR UPDATE` while waiting for membership,
and an invite INSERT needs `KEY SHARE` on that organization for its foreign key.
Invite operations take the organization `FOR SHARE`: this conflicts with role
changes' `FOR UPDATE`, but allows invite acceptance's membership foreign key to
take `KEY SHARE`. Otherwise acceptance holding the invite and revocation holding
the organization can deadlock. Other operations keep their organization
`FOR UPDATE` locks. Revocation authorizes before looking up the invite, including
before reporting an invalid invite identifier.

The mapping from role to capability lives in one place in `domain`, in code, not
in the database. **When roles change shape — several per person, new ones per
industry — only that mapping changes.**

- **Allow only.** There is no explicit deny. Deciding whether a team-level allow
  beats a channel-level deny is a problem to take on when it is needed, not before
- **No permissions granted directly to individuals.** An actor gets a role; a role
  has capabilities — for a person, a bot and an integration alike
- **Ask about an `actor`.** Capabilities belong to whoever acts, person or not;
  a token's scopes can narrow them without a second model

## Two access primitives

There are exactly two ways to ask whether someone may see something. **Every
route that reads or changes a protected resource uses at least one of them, and
says which.** Some use both — listing an organization's channels checks
organization membership, then channel access per channel.

Operational routes such as `/health` touch no protected resource and are outside
this model.

| Primitive | Decides access to |
|---|---|
| `is_organization_member(organization, actor)` | The organization's own resources: its teams, its members, its invites, its settings |
| `can_access_channel(channel, actor)` | A channel and everything in it: messages, history, live subscription, resolving who wrote what |

`can_access_channel` branches on the channel's scope:

| Scope | Allowed when |
|---|---|
| `team` | The actor is a member of **the channel**, **and**, if the actor has a membership in the channel's organization, **that membership is active**. An actor with no membership there — a member from outside — is admitted by channel membership alone (see the row-security note below). **A suspended or departed member of the organization is not admitted just because they are still listed in the channel** |
| `organization` | The actor is an **active member of the organization**. Nobody is listed individually |

**Callers never choose the branch.** The branch is inside the function.

### Why two, and why they must not be merged

A channel may hold members from outside its organization. That makes "may see this
channel" and "belongs to this organization" genuinely different questions, and
merging them fails in both directions:

| Merged how | What goes wrong |
|---|---|
| Channel contents guarded by organization membership | Legitimate outside members are locked out |
| Organization resources guarded by channel membership | **Another organization's data is visible** to anyone who is in the right channel |

The second failure passes ordinary tests.

## Something you cannot see does not exist

A resource the caller may not see is reported as **not found (404)**, and must be
indistinguishable from one that does not exist. Answering 403 confirms that it is
there.

## Row security is the last wall, not the first

Every query in repository code that reads or writes tenant-owned data **below the
root** filters by organization explicitly; a query on `organizations` itself is
scoped by its own `id`. Cross-tenant links (`channel_members`) are reached through
the channel they belong to. Global rows (`actors`, `users`, `sessions`) are
**read in a tenant context** — whose name to show in this organization — through a
membership; **identity lookups that have no tenant context**, such as signing in by
email or an actor that has left every organization, do not need one. **Leaving the
service is the one service-wide exception**: it reads the acting actor's own
organization memberships and channel listings by `actor_id` alone, because the
organizations it belongs to are what that read discovers. Row security admits only the
actor's own rows there, and every write that follows is scoped by the `organization_id`
each row names. **Invite acceptance is the other exception**: the pre-check and
conditional use look up a globally unique token hash before an organization is known.
`fukulow.invite_token_hash` admits that one row; every following write is scoped by
its returned organization. PostgreSQL row level security sits underneath as the wall
that holds when something reaches the tables another way.

- The application sets the acting actor's id, `fukulow.actor_id`, or the credential
  the request presented: `fukulow.session_token_hash`, `fukulow.sign_in_email`, or
  `fukulow.invite_token_hash`. Every setting uses `set_config(..., true)` inside
  the transaction. **No setting names an organization**; the database derives
  the actor's active organizations from membership rows
- **With no actor or credential set, no rows are visible.** Global `actors` admits
  self and every actor with a membership, of any status, in the acting actor's active organizations; `users` admits self or the one ASCII-case-insensitive
  sign-in email match; `sessions` admits self or the one token-hash match.
  `invites` also admits the one presented invite hash. Invite acceptance creates
  an actor and sets its id before inserting the user. The invite insert policy admits
  only an active member for that actor in the invite's
  organization, after a successful conditional increment in the same transaction.
  It checks the invite's kind, expiry, revocation and `xmin`; a trigger prevents
  no-op updates and fabricated initial counts from supplying that proof
- Inactive actors can reach their own organization, team and channel membership
  rows for departure, without seeing anyone else's rows. Audits and listing
  removals precede all status transitions. An outside actor leaving the service
  records the removal of each of its own listings in the channel's organization —
  the one audit it may write there; it gains no access to that organization.
  An inactive actor's own organization membership can only become `left`, with its
  display name cleared and its role unchanged
- **Passing row security is not permission.** Policies decide visibility;
  `authorize(conn, actor, organization, Capability)` decides what may be done
  inside the writing transaction
- The application role cannot bypass row security: it does not own the tables,
  is not a superuser, and has no `BYPASSRLS`. Every application table, including
  globals, has ENABLE and FORCE. Exactly two hardened definer functions read
  memberships as the migrator to avoid recursive policies; the application
  cannot assume that role. The test-only inspector bypasses FORCE, and the
  application refuses startup when `INSPECTOR_DATABASE_URL` is set

**Members from outside the organization cannot read through row security yet.**
The model admits them to team channels; the policies are derived from organization
membership alone, so today they get no rows. That is deliberate while connections
between organizations do not exist: **the gap fails closed** — an empty result,
never exposure — and a test pins it so that extending it is a conscious change.
The policies are extended when connections are built.
- **Nothing runs without an identity.** A job that needs one runs in one
  organization's context at a time

The enforcement details and how each is tested are in
[`../architecture/invariants.md`](../architecture/invariants.md).

## Not yet decided

- Further capabilities beyond `InviteMember`, and which roles hold them
- Who may post to an organization-scoped channel — this belongs to the channel
  (`everyone` or administrators only), not to a job title, and is not built yet
- How connections between organizations are requested and approved
