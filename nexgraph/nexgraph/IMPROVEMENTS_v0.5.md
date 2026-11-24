# nexgraph v0.5: PostgreSQL backend

Added: `src/pg.rs` (feature `postgres`, default on), `Database::open_postgres[_with]`, `PgOptions`, `refresh`, `verify_mirror`,
`rebuild_mirror`, `mirror_stats`; `Error::Backend`; CLI target `postgres://...`; `tests/pg.rs`; `examples/bench_pg.rs`; `POSTGRES.md`.
Changed: `db.rs` durability is now an enum (memory / file WAL / PostgreSQL); read-only handles refuse writes. The file backend behaves as before.

## Evidence (Linux sandbox, 1 core, PostgreSQL 16.15 on localhost, local disk)
* Tests: 58 pass (42 from v0.4 unchanged + 15 PostgreSQL + 1 doctest). The PG tests cover recovery, time travel, checkpoint chunking,
  writer lock, replicas, killed connection, corruption, foreign-writer fencing, awkward values, and a 300-commit randomized
  differential against the in-memory engine (state, time travel and mirror all compared). Disabling the node-delete cascade in the mirror made 3 tests fail.
* Benchmark (`bench_pg`, single runs, noisy): single-node commit median ~215-320 us (file WAL fsync ~90-115 us); with mirror ~245-460 us;
  `synchronous_commit=off` ~45 us / ~100 us with mirror. Bulk load 10k nodes + 80k edges: 0.19 s without mirror, ~3.0 s with it
  (in memory only: 0.12-0.16 s). Open from log or checkpoint ~0.09 s. verify_mirror 1.3-2.4 s, rebuild ~2.8 s.
  1-hop read: NQL 5 us vs SQL over mirror ~600 us (both indexed).
  fsync on this sandbox disk is unusually cheap; expect larger synchronous-commit costs on real servers.
* Caveat: the mirror makes bulk loads ~15x slower; use `--no-mirror` for large imports and `rebuild_mirror()` afterwards if needed.
