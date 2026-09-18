# HTTP contract

The API lives under `/api/v1`. Every HTTP error response has an empty body: only
its status communicates the failure, including 401, 403, 404, 409, 422 and 500.
A 422 never identifies an invalid field. WebSocket `error` frames are a separate
contract.

## Public origin and CSRF

`FUKULOW_PUBLIC_ORIGIN` is required with no default. It contains a scheme, host
and optional port, such as `https://chat.example.com:8443`. Credentials, paths,
a trailing slash, queries and fragments are refused at startup without echoing
the rejected value.

**It must be written the way a browser sends it**: a lower-case scheme and host,
and no port when it is the scheme's default. `https://Chat.Example.com` and
`https://chat.example.com:443` are refused at startup. The Origin check compares
bytes, so accepting them would start a server that refuses every same-origin
request that changes state.

HTTPS is required because browsers refuse the Secure session cookie on insecure
origins. Development permits `http://localhost`, `http://127.0.0.1` and
`http://[::1]`, with any port. Chromium was measured to accept the cookie on
loopback HTTP; Firefox and Safari were not measured. A browser refusing that
cookie requires a local HTTPS origin. Plain HTTP on a LAN address is not allowed.

Every POST, PUT, PATCH and DELETE must carry exactly one `Origin` header equal
to `FUKULOW_PUBLIC_ORIGIN`. The same rule applies to the WebSocket handshake when
that route arrives. Missing, duplicate or unequal origins return 403 before any
handler runs. GET routes do not change state and need no Origin, except for a
WebSocket upgrade. `SameSite=Strict` and this Origin check are both required.

Checks run in this order; the first failure wins:

1. Origin (403).
2. Credential, on routes requiring authentication (401).
3. Body (422).
4. Operation (including invalid sign-in credentials: 401).

## Browser session

A successful sign-in returns 204 and exactly one `Set-Cookie`:

```text
__Host-fukulow_session=<opaque token>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=1209600
```

There is no `Domain` or `Expires` attribute. `__Host-` requires Secure, Path=/
and no Domain, preventing sibling subdomains from setting the session cookie.
The cookie persists across browser restarts. Its `Max-Age` is the same nominal 14 days
as the session, and **the database's expiry is authoritative**: the browser starts
counting when the response arrives, slightly after the expiry was set, so a cookie
it still holds can meet an expired session and get 401. Sessions last 14 days from sign-in;
use never extends their expiry. The database stores only a SHA-256 hash of a
256-bit cryptographically random token. Revoked, expired, unknown and missing
sessions all produce the same empty 401 response.

Sign-out revokes only the session carried by the request. Its clearing cookie
has the same name, HttpOnly, Secure, SameSite=Strict and Path=/, with Max-Age=0
and no Domain or Expires. Other sessions stay valid.

A browser session is never a bearer token. Session tokens in paths, query
strings or Authorization headers are not accepted. Future non-browser bearer
credentials are a separate credential type resolving to the same actor.

## Routes

| Route | Input | Success | Errors |
|---|---|---|---|
| `POST /api/v1/sessions` | JSON object with string `email` and `password`; neither has a default | 204 and session cookie | 403, 422, 401, 500 |
| `DELETE /api/v1/sessions/current` | Session cookie; no body required | 204 and clearing cookie | 403, 401, 500 |
| `GET /api/v1/me` | Session cookie | 200 and identity JSON | 401, 500 |

Sign-in parses JSON regardless of Content-Type. A non-object, malformed JSON,
a missing or non-string email or password, or an email containing U+0000 returns
422. Email matches ignoring ASCII case. Wrong passwords and unknown email addresses
are indistinguishable; unknown addresses still incur one password verification.
**Every credential failure is 401, decided before the session token is made**: a
token is generated only once the credential is accepted, so a failing random number
generator answers 500 only for a correct credential. Sign-in stores no password
hash. A person who has left cannot sign in and has no sessions.

`GET /api/v1/me` returns:

```json
{
  "actor": {
    "id": "019f0000-0000-7000-8000-000000000001",
    "type": "human",
    "display_name": null
  },
  "user": { "email": "person@example.invalid" }
}
```

The id is a UUID string, and display_name is a string or null. Actor types are
`human`, `bot`, `integration` and `system`. A non-person has no `user` key. Only
people can hold browser sessions; non-person route access awaits non-browser
credentials. Authentication produces an actor in one extractor. Handlers use
that actor; only sign-out and the future WebSocket handler consume the private
current-session hash.

## Organization invites

| Route | Input | Success | Errors in decision order |
|---|---|---|---|
| `POST /api/v1/organizations/{organization_id}/invites` | Session cookie; JSON object with required `expires_in` and optional `max_uses` | 201 with `id`, `token`, `link`, `expires_at`, `max_uses` | Origin 403; session 401; body 422; active membership 404; `InviteMember` 403; operation 500 |
| `DELETE /api/v1/organizations/{organization_id}/invites/{id}` | Session cookie | 204; already revoked also succeeds without another audit | Origin 403; session 401; active membership 404; `InviteMember` 403; invite in this organization 404; operation 500 |
| `POST /api/v1/invite-acceptances` | JSON object with `token`, `email`, `password`, `display_name` | 204, no body or session | Origin 403; body/token shape 422; unusable token 404; remaining fields 422; unusable at conditional use 404; email already registered 409; operation 500 |

A path identifier that is not a UUID is answered exactly as one that exists nowhere:
404 at the step that looks it up, with no body. UUID syntax is the hyphenated
36-character form, in either case. Thus a malformed organization id does not take
precedence over a missing session or invalid creation body. A malformed invite id
is checked after organization membership and capability.

`expires_in` is an integer number of seconds, from 300 through 604800 (five minutes
through seven days). `max_uses` is an integer from 1 through 100; only omission
defaults to 1. Fractions, floating-point JSON numbers, strings and null are refused.
`expires_at` is RFC 3339 in UTC with a `Z` suffix, and ids are JSON strings.

Acceptance parses JSON regardless of Content-Type. The body must be an object with
a string `token`. A read-only pre-check then rejects every unknown, expired,
revoked, exhausted or non-organization invite with the same empty 404, before
reading any other field. Unusable tokens incur no password hash. With a usable
token, the remaining fields must be strings:

- `email`: 3–254 bytes, exactly one `@`, no Unicode whitespace or U+0000. Not trimmed.
- `password`: at least eight Unicode scalar values and at most 1024 bytes. Not
  trimmed; U+0000 is permitted because only its hash is stored.
- `display_name`: trim Unicode whitespace at both ends, then require 1–80 Unicode
  scalar values and no U+0000. Store the trimmed value.

Password hashing runs outside all transactions on a blocking worker. The acceptance
transaction conditionally increments `used_count` and alone decides whether a place
remains. A duplicate email rolls back the use and every insert. Acceptance creates
an active member and records the invite in the membership audit; it does not sign
anyone in. The 409 reveals registration only to a usable-token holder, an accepted
v0.1 tradeoff; rate limiting is still planned.

The returned link is `{FUKULOW_PUBLIC_ORIGIN}/invite#<token>`. Only the configured
origin is used, never Host, X-Forwarded-Host or Forwarded. The token is returned once
at creation and stored only as a hash. A URL fragment never reaches the server;
the client reads it and posts the token in the acceptance body. No route accepts a
token from a path or query string. Sending email and admitting an existing account
to another organization are outside this contract.
