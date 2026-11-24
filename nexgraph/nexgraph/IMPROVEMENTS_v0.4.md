# nexgraph v0.4: ideas taken from Polars

Polars is a columnar dataframe engine, not a graph database, so only its execution ideas transfer. The [polars landing page](https://pola.rs/) gives almost no internals ("plans every query and strips out the wasted work"), and the official docs subpages were not reachable from this environment. The technique list below comes from the [docs.pola.rs overview](https://docs.pola.rs/) and a third-party write-up, [Polars internals (DEV Community)](https://dev.to/gowthampotureddi/polars-internals-query-optimizer-lazy-frame-arrow2-why-its-10x-faster-than-pandas-3b8h), whose speedup figures I did not rely on.

## Polars ideas and what nexgraph did with them
| Polars idea | v0.4 change |
|---|---|
| Columnar buffers, dictionary-encoded (categorical) strings | A flat array of 40-byte records, one per entity (`Cm`): visibility timestamps, dictionary-encoded label id, and for binary edges both endpoints. Edge/target label checks and endpoint lookups no longer chase entity → version chain → `Arc<Body>` → member vectors. Old snapshots and multi-member hyperedges fall back to the version chain, so time travel and hyperedge semantics are unchanged. |
| Projection pushdown (read only needed columns) | A one-hop `MATCH ... RETURN <keys of a>, count(*)` binds no variables for the edge or target and never builds rows; only node `a` is touched. |
| Aggregation without materialising rows (and Kuzu-style factorisation) | Matches are counted per start node (`View::hop_count`) and fed to the aggregator once, weighted by the count. |
| Parallel scans on a work-stealing pool | The per-start-node counts run on scoped std threads (no new dependency) for scans over 20,000 nodes. This sandbox has 2 cores, so the thread gain here is at most 2x and was not measured separately. |
| Slice pushdown (top-K instead of a full sort) | `ORDER BY ... LIMIT k` selects the first `skip+limit` rows with `select_nth` and sorts only those; the original position is the last tiebreak so results equal a stable full sort. |

## Measured (same machine, data and harness as the earlier report)
| workload | v0.3 | v0.4 | best other engine |
|---|---|---|---|
| 1-hop | 11.5 µs | 7 µs | SQLite 2.7 µs |
| 2-hop | 67 µs | 22–23 µs | SQLite 11.3 µs |
| reach3 (`*1..3`) | 806 µs | 600 µs | SQLite 315 µs |
| full scan, 800k edges | 577 ms | 22–33 ms | Kuzu 6.5 ms |
| group-by degree | 776 ms | 192 ms | DuckDB 20 ms |
| shortest path | 1.2 ms | 1.2 ms | (nexgraph fastest) |
| PageRank | ~231 ms | 219–240 ms | rustworkx 185 ms |

## What did not change, and limits
* The factorised path only applies to a single one-hop path pattern with no property filters and no `WHERE`, where every aggregate is `count(*)` and every other `RETURN` item (and `ORDER BY`) depends only on the start node. Anything else runs the general streaming executor, which also benefits from the flat metadata but is still row-at-a-time. `tests/v03.rs` checks the fast path against the generic path and an independent reference, including deletes, time travel, directions and undirected patterns.
* Group-by is still about 9x slower than DuckDB: the remaining cost is per-group allocation in the aggregator (100k groups here), not edge traversal.
* Variable-length paths and PageRank are unchanged in shape; PageRank still builds its adjacency from the version chains on every call.
* Storage is still row-oriented per entity; the flat array duplicates a few fields (memory +40 bytes per entity). Properties are not columnar.
