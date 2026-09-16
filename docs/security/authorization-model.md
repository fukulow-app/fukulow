# Authorization model

Who may see and do what, and where that is decided.

**This is the model, not a permission list.** The set of capabilities is built up
as features land; a table of them written ahead of the features would describe
things that do not exist. What is fixed here is the grammar those capabilities
are written in.

## The tenant is the organization

Every piece of data belongs to exactly one organization, **except users**, who are
global: one person can belong to several organizations and keeps the same
identity across them.

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
if can(&actor, Capability::InviteMember) { ... }
```

The mapping from role to capability lives in one place in `domain`, in code, not
in the database. **When roles change shape — several per person, new ones per
industry — only that mapping changes.**

- **Allow only.** There is no explicit deny. Deciding whether a team-level allow
  beats a channel-level deny is a problem to take on when it is needed, not before
- **No permissions granted directly to individuals.** A person gets a role; a role
  has capabilities
- **An `actor`, not a `user`.** Something other than a person — an integration, a
  bot — may act one day, and the signature should not have to change

## Two access primitives

There are exactly two ways to ask whether someone may see something. **Every
route uses one of them, and says which.**

| Primitive | Decides access to |
|---|---|
| `is_organization_member(organization, actor)` | The organization's own resources: its teams, its members, its invites, its settings |
| `can_access_channel(channel, actor)` | A channel and everything in it: messages, history, live subscription, resolving who wrote what |

`can_access_channel` branches on the channel's scope:

| Scope | Allowed when |
|---|---|
| `team` | The actor is a member of **the channel**. Organization membership is **not** a condition — outside members are allowed |
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

Every query in repository code filters by organization explicitly. PostgreSQL row
level security sits underneath as the wall that holds when something reaches the
tables another way.

- The application sets **only** the acting user's id, per transaction, with
  `SET LOCAL`. **Which organizations that user may see is derived by the database**
  from their memberships. Letting the application declare the organization would
  trust it exactly as much as the `WHERE` clause already does
- **With no user set, no rows are visible.** Forgetting to set it produces an empty
  result, not another organization's data
- No role can bypass row security, and the application does not own its tables
- **Nothing runs without an identity.** A job that needs one runs in one
  organization's context at a time

The enforcement details and how each is tested are in
[`../architecture/invariants.md`](../architecture/invariants.md).

## Not yet decided

- Which capabilities exist, and which role holds each
- Who may post to an organization-scoped channel — this belongs to the channel
  (`everyone` or administrators only), not to a job title, and is not built yet
- How connections between organizations are requested and approved
- How users are protected by row security — they belong to no organization, so the
  organization-based policy does not apply. Decided and measured in #10
