# nexgraph v0.3: benchmark-driven improvements

v0.2's benchmark (100k nodes, 800k edges, same data and query samples for every engine) showed nexgraph winning on indexed lookups and shortest path but far behind on full scans (about 200x slower than Kuzu), group-by (about 36x slower than Kuzu, 79x slower than DuckDB) and PageRank. This note records what the faster engines do, what was adopted, and what was not.

## What the top performers do (sources: [Kuzu technical report](https://cs.uwaterloo.ca/~ssalihog/papers/kuzu-tr.pdf), [FalkorDB design](https://docs.falkordb.com/design))
| Engine | Techniques |
|---|---|
| Kuzu | Columnar node storage; CSR adjacency lists in both directions with parallel edge-property storage; vectorized, factorized operators; morsel-driven parallelism; ASP/S-join with sideways information passing; multiway worst-case-optimal joins for cycles |
| DuckDB / SQLite | DuckDB: vectorized columnar execution and hash aggregation. SQLite: B-tree access paths and tiny per-row overhead |
| FalkorDB | Adjacency as sparse GraphBLAS matrices; a hop is a sparse matrix product; per-relationship and per-label matrices |

## Where nexgraph's time went (callgrind, plus wall-clock)
* Every matched row cloned a `HashMap<String, Value>` and materialised the whole row set before `RETURN` ran.
* `View::incident` allocated a `HashSet` and a `Vec` per node visited; `cross` allocated a `Vec` per edge.
* Aggregation keys were built with `format!("{key:?}")` and hashed with SipHash.
* Undirected adjacency was walked for directed hops (half the edge fetches were wasted).
* `algo.shortestPath` was a one-sided BFS over `HashMap`s; PageRank used nested `Vec<Vec<(usize,f64)>>` and a `HashMap` id lookup.

## Changes in v0.3
1. **Streaming executor.** `MATCH ... RETURN` is fused: matches flow straight into a projector/aggregator and are never materialised. `LIMIT` without `ORDER BY`/aggregation/`DISTINCT` stops the traversal early.
2. **No per-row environment cloning.** Bindings are an association list with `Rc<str>` names; they are added and undone by truncation during the depth-first match.
3. **Hash aggregation.** Group and `DISTINCT` keys are type-tagged byte keys (`Int(1)` and `Float(1.0)` stay distinct) in a fast-hash map; aggregates (`count`, `sum`, `avg`, `min`, `max`, `collect`, with `DISTINCT`) are streaming accumulators.
4. **Allocation-free adjacency.** `incident_iter` / `cross_each` replace the per-node `HashSet`/`Vec`. Incident lists are de-duplicated at insert time instead of at every read.
5. **Directed adjacency lists.** Each node keeps `outs` and `ins` lists next to `incident`, so a directed hop touches only the relevant edges (the idea behind Kuzu's forward/backward CSR, kept inside the MVCC version-chain design).
6. **Bidirectional BFS** for `algo.shortestPath` over dense per-id arrays; the whole level is scanned before stopping, so the path stays shortest.
7. **PageRank on CSR** with precomputed transition weights and a dense id table.

## Not adopted (and why)
* Columnar storage, vectorized batches, morsel-parallel scans, factorized intermediates, worst-case-optimal joins: these are a new storage and execution engine, not a patch. Full scans therefore remain far slower than Kuzu and DuckDB.
* GraphBLAS matrices: they would not help the MVCC and hyperedge model, and the biggest remaining cost is pointer chasing (entity → version → `Arc<Body>` → member vectors), not arithmetic.
* The most promising next step is a compact per-edge metadata array (label id, endpoints, visibility timestamps) so a scan never touches the heap-allocated bodies.

## Verification
* 41 tests pass (38 from v0.2 plus 3 new in `tests/v03.rs`: bidirectional BFS against a reference BFS on a random graph with an undirected hyperedge; streaming-aggregate semantics including empty inputs, mixed numeric sums, DISTINCT and early LIMIT; directed adjacency with self-loops and a node in both tail and head).
* The benchmark harness cross-checks every engine's results against nexgraph's; all cells agree after the changes.
* `examples/prof_scan.rs` is the small profiling driver used for the callgrind runs.
