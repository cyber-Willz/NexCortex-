# nexgraph v0.6: TLS, authentication, multi-tenant security

Added: `src/tls.rs` (rustls for PostgreSQL), `src/security/` (control plane, tenant manager, sessions, authorization),
`src/server.rs` + `src/bin/nexgraph-server.rs` (HTTPS API, feature `server`), `SECURITY.md`, tests `tls.rs`, `security.rs`, `server.rs`,
`tests/fixtures` (test-only certificates, `gen_certs.sh`). New `PgOptions`: `require_tls`, `tls`; `Database::pg_session_info()`, `grant_mirror_select()`.
New error variants `Auth`, `Denied`. Backend errors now include the underlying cause (TLS alerts, bad configuration).
Minimum Rust version 1.85. Cargo features: `postgres` (now includes TLS), `security` (default), `server` (opt-in).

## Evidence (Linux sandbox, PostgreSQL 16.15; one plaintext server and one TLS-only server)
* 81 tests pass: 42 from v0.4 (unchanged), 15 PostgreSQL backend, 4 TLS, 11 security/isolation, 4 HTTPS server, 4 security unit tests, 1 additional doctest.
  (`cargo test --release --features server -- --test-threads=1` with `NEXGRAPH_TEST_PG_URL` and `NEXGRAPH_TEST_PG_TLS_PORT`; the integration tests skip without them.)
* The TLS tests assert the *reason* for each refusal: `UnknownIssuer` for an untrusted CA, "not valid for name" for a mismatched host name; a TLS-only server rejects plaintext;
  `require_tls` refuses a server without TLS; the API negotiates TLS 1.3 and gives no service on the port to plain HTTP or to clients that do not trust the certificate.
* Mutation checks (break the code, expect failures): readers allowed to write -> 2 tests fail; all tenants mapped to one schema -> 6 fail; token expiry ignored -> 1 fails.
  A write whose audit entry cannot be stored is not executed (test), reads continue.
* Bugs the tests found and that are fixed: quota blocked deletes at the limit; tenant schema names collided across control planes; dropping a synchronous
  PostgreSQL client inside an async context aborted the process (clients are now dropped on a helper thread, which also makes `Database` safe to drop in async code).

## Not measured / not done
Throughput and latency of the authenticated path were not benchmarked. No external security review. See the limits list in `SECURITY.md`.
