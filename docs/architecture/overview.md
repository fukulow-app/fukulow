# Architecture overview

How the code is divided, and which way the divisions may depend on each other.

## Crates

| Crate | Responsibility | Must not |
|---|---|---|
| `protocol` | The wire format: frames and their serialisation | Know about storage, or depend on any type from `domain` |
| `domain` | Types and rules: organization, team, channel, message, authorization; the canonical name of each enumerated value | Choose a serialization format, or depend on `serde` |
| `db` | Persistence. **The only place that writes SQL** | Be bypassed: callers do not write SQL, and organization-scoped reads go through here |
| `auth` | Authentication, sessions, invite tokens | Store or log a token in plaintext |
| `realtime` | Subscriptions and fan-out | Expose a `tokio` channel type, or its lag error, in its public API |
| `app` | The binary: wiring, configuration, shutdown | — |

`domain` owns canonical names, not formats. `db` adopts them as stored values, held
equal to the database's `CHECK` by a test; whether the wire uses them is for the
caller that maps to `protocol` (message frames arrive with #6).

Each library crate's `//!` documentation says the same, so the boundary is
visible from the code as well. `app` is the binary and has none.

## Dependency direction

```mermaid
graph TD
    app --> protocol
    app --> domain
    app --> db
    app --> auth
    app --> realtime
    db --> domain
    auth --> domain
    auth --> db
    realtime --> domain
    realtime --> protocol
```

`protocol` and `domain` depend on nothing in this workspace.

**The direction is declared in the manifests, and cargo enforces only part of
it.** Cargo rejects a cycle, so *reversing an existing edge* fails to build —
`domain` depending on `db`, for example, because `db` already depends on `domain`.

**Cargo does not reject a new edge that forms no cycle.** `domain` depending on
`protocol` builds without complaint, and is still forbidden. **Enforcement is in
place:** [`deny.toml`](../../deny.toml)'s `[bans]` list holds the allowed direct
parents of each library crate. The required `audit` job runs `cargo deny check
licenses bans sources` to enforce it, including dev-dependencies and optional or
feature-gated dependencies through `[graph] all-features = true`.

An allowed edge need not be present. A new workspace member must be added to the
list when it is introduced: a member with no entry is unrestricted as a dependency.

## Where types are documented

**In rustdoc, not here.** A list of important types in Markdown is the fastest
document to fall behind the code. `cargo doc --open` is the reference.

## Request flow

HTTP middleware checks `Origin` before extraction on state-changing requests.
`Authenticated` reads the session cookie, hashes its token and resolves it to an
actor through `db`. Handlers use that actor in their own transactions; sign-out
also consumes the private current-session hash. Password verification runs on a
blocking worker while the sign-in transaction stays open. `/health` touches no
protected resource. The [HTTP contract](../HTTP.md) specifies carriers and errors;
message and WebSocket flow follows when those routes arrive (#6).

## Where things are decided

| What | Where |
|---|---|
| Rules that must always hold | [`invariants.md`](invariants.md) |
| Who may see and do what | [`../security/authorization-model.md`](../security/authorization-model.md) |
| What words mean | [`../domain/glossary.md`](../domain/glossary.md) |
| Work in progress, what is next | GitHub issues |
