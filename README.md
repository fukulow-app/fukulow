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
| **channel** | Where a conversation happens. Lives inside a team. |
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

**No database is needed yet.** `compose.yaml` is here for the schema that comes
next; starting it now would run PostgreSQL for nothing.

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
