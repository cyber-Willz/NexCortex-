# nexgraph v0.9: improvements found by running a 100k-event cyber-attack dataset

Dataset: `cybersecurity synthesized data.csv`, 100,000 attack events (attacker/target IP, attack type, target system, outcome,
ISO timestamp, data compromised, duration, tool, user role, country, severity, industry, response time, mitigation). Every IP appears
in about one event, so as a binary graph it is 99,996 disconnected pairs; the information is in the *rows*, not in the IP links.

## What the dataset exposed (all fixed, each with a regression test in `tests/v09.rs`)

| # | Found in v0.8 | Fix in v0.9 | Before -> after (this sandbox, release, 1 core) |
|---|---|---|---|
| 1 | `spectral.cluster/fiedler/embedding` **panicked** ("Matrix index out of bounds") on a graph with ~100k components: Lanczos returned fewer than `k` eigenvectors and the caller indexed `k` columns | `decompose` validates the result; dense fallback when small, otherwise a clear error that names the cause (many components) | crash -> `Error::Query` explaining the cause |
| 2 | `spectral.*` on a hypergraph with many **parallel hyperedges** (100k events over 51 dimension values) tried to allocate **80 GB** and aborted: `dense_normalized_laplacian` builds `m x m` diagonal matrices (`m` = hyperedges). A second quadratic cost hid in `incidence_weight` (O(degree) per call) | `algo::spec::dense_laplacian`: same matrix (unit-tested equal to the reference to 1e-12 on random weighted hypergraphs with duplicates), `O(n^2 + sum|e|^2)`, no `incidence_weight` | abort after >8 min / 80 GB -> **125 ms** (100k events); 16k events 2.3 s -> 14 ms |
| 3 | `MATCH (a:Host) WHERE a.ip = '...'` ignored the property index (only inline `{ip: '...'}` used it): every point lookup scanned all nodes | executor rewrites top-level `AND` equality conjuncts `var.key = literal/$param` into the node pattern when `(label, key)` is indexed; the WHERE still runs, so results are unchanged. Null/list values keep scan semantics. Differential test: indexed == unindexed on 11 query shapes incl. int/float/string/null/missing/OR/NOT | 300 point queries on 200k nodes: **38.8 s -> 0.43 s** (130 ms -> 1.4 ms each) |
| 4 | Timestamps in CSV stay text (`2024-04-03 11:13:15.083419`); `cyber.beaconing`, `stats.timeseries`, `stats.spikes` then returned **empty results with no warning** | those procedures read numbers (epoch seconds) *or* ISO 8601 strings (`timeutil::epoch_seconds`); if the property exists but nothing parses they raise an error quoting the first value | silent 0 rows -> correct buckets / explicit error |
| 5 | `CALL ... YIELD *` was a syntax error | accepted (all columns) | |

## New: one hyperedge per row (`ingest::import_csv_hyperedges`, shell `.import hyper`)

```rust
let m = [("attack_type","AttackType"), ("industry","Industry"), ("location","Country"), ("attacker_ip","Ip"), ("target_ip","Ip")];
import_csv_hyperedges(&db, &HyperCsv { edge_label: "ATTACK", members: &m, key_prop: "name", weight_col: None, time_cols: &["timestamp"] }, &csv)?;
```
Each row becomes an undirected hyperedge over the nodes named by `members` (found or created by value, property index created automatically);
other cells become typed edge properties; `time_cols` are stored as epoch seconds. This keeps the co-occurrence structure that star-shaped
binary edges lose, so `spectral.*` sees attack-type/industry/country co-occurrence. Rows with < 2 distinct members, an unparsable timestamp or a bad
weight are skipped and counted *before* any node is created (no orphan nodes). Re-importing reuses existing member nodes.
Shell: `.import hyper ATTACK file.csv name attack_type:AttackType,industry:Industry timestamp`.

## Evidence
* `tests/cyber_events.rs` (needs `NG_CYBER_CSV`, `NG_CYBER_EXPECTED`): the real 100,000-row file imported as hyperedges; 15 aggregates compared with values computed independently in pandas (counts per attack type, co-occurrence counts, GB sums, a conditional average, severity/outcome filters, a January-2024 time window, total volume): all equal. Import 1.9 s (200,047 nodes, 100,000 hyperedges); typical hyperedge-pattern aggregate 100-170 ms; `spectral.cluster(3)` over the 51 dimension values 135 ms.
* `tests/lifecycle.rs` on the same data as a binary-edge graph (199,996 nodes, 100,000 edges): all 13 phases pass in 36 s (previously the query phase alone took 39 s).
* Full suite (rustc 1.97): 14 unit + 12 analytics/cyber + 13 bi + 17 db + 9 ingest + 11 integrated + 4 v03 + 6 v0.9 + lifecycle + HIBP + cyber_events, all pass; PostgreSQL/TLS/security tests skip without a server as before; `--features server` builds.
* Not mutation-tested this round beyond reproducing bugs 1-4 on v0.8 before fixing them.

## Limits (honest list)
* Index pushdown covers equality conjuncts on node patterns only; no range predicates (`>`), no edge-property indexes, no cost-based planner.
* Spectral analysis of a graph with thousands of disconnected components still fails (now with a clear error); restrict to one component first. No automatic "largest component" mode.
* The dataset is synthetic and near-uniform (every attack type, industry and country is equally likely), so the analytics show no real signal; this release validates correctness and scale, not detection quality. `cyber.beaconing` correctly finds nothing: each IP pair occurs once.
* `.import hyper` is undirected hyperedges only; no streaming import (whole file in memory).
* The shared `spectral_hypergraph` crate is still unmodified; the quadratic helpers are bypassed in nexgraph rather than fixed upstream.
