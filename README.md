# fukulow

**A team chat server you can host yourself.** Written in Rust.

> **Status: early. Nothing works yet.**
> The repository holds the license, the contribution rules and the development
> setup. The first release target is a single vertical slice — two people in a
> browser exchanging messages that survive a reload — not a feature set.

## What it is

fukulow is built around the shape a company actually has, rather than a single
flat team:

| Concept | What it is |
|---|---|
| **organization** | A company or other body. Everything below it belongs to one. |
| **team** | A grouping inside an organization. People are placed into the teams they belong to. |
| **channel** | Where a conversation happens. Owned by a team, or directly by the organization — a company-wide channel belongs to no team. |
| **direct** | A one-to-one or small-group conversation. |

A person is not owned by an organization. One account can belong to several,
and can keep belonging after a role ends — so leavers, contractors and
part-time staff do not need a second identity.

The interface is Japanese first. The names above are the identifiers used in
the code, the API and this documentation; the words shown on screen live in one
translation file and are not hard-coded.

## Running it

Requires [Rust](https://rustup.rs/); the pinned toolchain in
`rust-toolchain.toml` is installed for you on first build. Linux or macOS —
shutdown is handled through Unix signals, and Windows is not a supported host.

```bash
cp .env.example .env
docker compose up -d postgres
set -a; . ./.env; set +a
cargo install sqlx-cli --version 0.9.0 --locked --no-default-features --features postgres,rustls
cargo sqlx migrate run --database-url "$MIGRATOR_DATABASE_URL"
cargo run -p app
```

```bash
curl -i localhost:8080/health
```

```
HTTP/1.1 200 OK
content-type: application/json

{"status":"ok"}
```

`FUKULOW_PUBLIC_ORIGIN` is required, with no default. Set it to the browser's
origin, such as `https://chat.example.com`, including a port when needed. Paths,
trailing slashes, queries, fragments and credentials are refused. Startup requires
HTTPS because the `Secure` session cookie cannot work on insecure LAN origins.
Development may use `http://localhost`, `http://127.0.0.1` or `http://[::1]`, with
any port. Chromium was measured to accept the cookie on loopback HTTP; Firefox
and Safari were not measured. A browser that refuses it on `http://localhost`
needs a local HTTPS origin. See [the HTTP contract](docs/HTTP.md) for sign-in,
the persistent session cookie and the required `Origin` header.

`DATABASE_URL` is required. Startup opens a pool and checks PostgreSQL with a
round trip; failures name the variable without disclosing its value. The server
connects as `fukulow_app` and never runs migrations. The migrator owns the tables
and uses the separate `MIGRATOR_DATABASE_URL`.

`docker/init/001-roles.sql` runs automatically only for a new Docker volume. For
an existing development volume, apply it once before migrating:

```bash
docker compose exec -T postgres psql -U fukulow -d fukulow_dev < docker/init/001-roles.sql
```

For self-hosting, run the following once as the cluster administrator in the target
database, replacing the development passwords and database name first. Neither role
is a superuser or has `BYPASSRLS`. Only the migrator needs `CREATEDB`, for isolated
test databases; it can be omitted on a production-only migration account.

```sql
-- Development credentials only; self-hosters must replace both passwords. gitleaks:allow
CREATE ROLE fukulow_migrator LOGIN PASSWORD 'fukulow_migrator' NOSUPERUSER NOBYPASSRLS CREATEDB NOCREATEROLE;
-- The runtime cannot create objects or assume the migration role. gitleaks:allow
CREATE ROLE fukulow_app LOGIN PASSWORD 'fukulow_app' NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE;
GRANT CONNECT ON DATABASE fukulow_dev TO fukulow_migrator, fukulow_app;
GRANT USAGE, CREATE ON SCHEMA public TO fukulow_migrator;
GRANT USAGE ON SCHEMA public TO fukulow_app;
```

Migrations grant privileges explicitly per table and per updatable column. They
fail if the roles have not been created. Do not use default privileges. Reverting
the initial schema destroys its data; take a backup before a deliberate rollback.

```bash
cargo sqlx migrate revert --database-url "$MIGRATOR_DATABASE_URL"
cargo sqlx migrate run --database-url "$MIGRATOR_DATABASE_URL"
DATABASE_URL="$MIGRATOR_DATABASE_URL" cargo sqlx prepare --workspace -- --all-targets
DATABASE_URL="$MIGRATOR_DATABASE_URL" cargo sqlx prepare --workspace --check -- --all-targets
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
```

Tests require both URLs and create disposable `fukulow_test_*` databases on the
local development server. Operations and privilege tests connect as the application
role; schema inspection and audit assertions connect as the migrator. A failing
test can leave its disposable database for diagnosis. Never point tests at a
production server.

The `build` job compiles with `SQLX_OFFLINE=true` and no database. The `database`
job uses PostgreSQL 17, exercises migrations on empty and populated databases,
checks the actual server and migration count, runs all tests, and checks `.sqlx/`.
After this change merges, a repository administrator must add `database` to the
required checks on `main`; a workflow cannot change branch protection itself.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md). Commits need a `Signed-off-by` line
(DCO); there is no CLA.

## Security

Please do not open a public issue for a vulnerability. Use GitHub's
[private vulnerability reporting](https://github.com/fukulow-app/fukulow/security/advisories/new).
See [SECURITY.md](SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you shall be dual licensed as above, without
any additional terms or conditions.
