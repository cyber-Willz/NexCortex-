# nexgraph v0.7: data analytics and cyber-security

Added: `src/netgraph.rs` (shared projection + robust statistics), `src/analytics.rs`, `src/cyber.rs`, `src/ingest.rs` (feature `ingest`, default; adds `serde_json`),
CSV/JSON export on `QueryResult`, `stdev()`/`median()` aggregates, shell commands `.mode`, `.import`, `.drift`, `examples/bench_v07.rs`, tests `analytics_cyber.rs`, `ingest.rs`.
The multi-tenant procedure allow-list now includes `stats.` and `cyber.` (all read-only).

## Procedures
| | |
|---|---|
| `algo.degree/betweenness/closeness/kcore/triangles/scc/articulation/louvain/similarity` | graph metrics; optional trailing edge-label filter; `betweenness`/`closeness` take a sample size |
| `stats.describe/outliers/histogram/correlation/timeseries/spikes` | descriptive statistics over node/edge properties; outliers use median/MAD (robust z) |
| `cyber.reachable/attack_paths/chokepoints/propagate_risk/fanout/beaconing/volume/ioc` | see the table in `src/cyber.rs` |
| `nexgraph::drift(db, ts_old, ts_new)` | new/removed nodes, novel links between pre-existing nodes, link Jaccard |
| `ingest::import_csv_nodes/import_csv_edges/import_events` | CSV (typed, upsert by key) and Suricata EVE / Zeek JSON; one transaction per import; bad rows skipped and counted |

Conventions: edge `weight` is attacker *effort* for `attack_paths` (must be > 0) and *success probability* in [0,1] for `propagate_risk`; timestamps are epoch seconds.

## Evidence (rustc 1.97)
* 21 new tests (12 analytics/cyber, 9 ingest). Each procedure is compared with a brute-force reference: all-pairs betweenness, naive k-core peeling, O(n^3) triangles, reachability-based SCC, remove-and-count cut vertices, exhaustive simple-path enumeration (Yen's k cheapest), Louvain modularity equal to the planted optimum on a ring of cliques.
* Mutation checks (break the code, expect failures): chokepoints ignoring targets, Yen edge banning, k-core peel condition, wrong edge probability, Brandes predecessor test, leading-zero inference: each made at least one test fail.
* Bug found by the tests: label propagation merged two cliques joined by one edge (tie-breaking); replaced by Louvain.
* Existing suites unchanged and passing (lib 13, db 17, v03 4; PostgreSQL/TLS/security/server tests skip without a PostgreSQL server).

## Benchmark (release, 1 core, 100k nodes / 800k random edges; `examples/bench_v07.rs`)
degree 0.67 s, k-core 0.45 s, triangles 0.66 s, SCC 0.47 s, articulation 0.41 s, **Louvain 24.7 s**, betweenness (200 sampled sources) 4.2 s, chokepoints (1000 entries) 18.7 s, fanout 0.41 s, beaconing 0.91 s, volume 0.43 s, describe 35 ms, spikes 0.34 s. Time includes building the projection on every call (no cached projection).

## Not done / limits
* Louvain and exact betweenness are slow at this size; no cached projection; no incremental maintenance. All-sources algorithms refuse work above a budget (4e9) and ask for a sample.
* `beaconing`, `fanout`, `volume` are statistical leads; legitimate periodic traffic (updates, monitoring) is flagged too. Parallel duplicate edges count once in `drift`.
* `attack_paths` ranks by cost and filters by `maxHops` afterwards. Very large undirected hyperedges expand to s(s-1) arcs.
* Importers model flow/alert/dns/http/tls (Suricata) and conn/dns (Zeek) only; file paths with spaces are unsupported in the shell. No streaming import (whole file in memory).
* Detection quality was tested on synthetic data only, not on real traffic. No external security review.
