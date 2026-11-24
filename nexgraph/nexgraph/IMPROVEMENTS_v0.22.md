# nexgraph v0.22: range queries from the property index, and an enforced directory lock

Two gaps that earlier documents listed as limits, chosen because each is a correctness or an order-of-magnitude issue rather than a feature.

## Added
* **Range pushdown.** `MATCH (n:L) WHERE n.k > x`, `>=`, `<`, `<=` (literal or `$param` bound, property on either side, several conjuncts joined by `AND`) now take their start
  candidates from the `(L, k)` property index through the new `View::lookup_range`, exactly as equality already did. The property index was already an ordered `BTreeMap`; only the query layer
  and a range accessor were missing. The WHERE clause still runs on the candidates, so the result can only ever differ by speed.
  * Only top-level `AND` conjuncts qualify (`OR` / `NOT` are never pushed down).
  * Semantics follow the query language: numbers compare with numbers (Int and Float together), strings with strings, booleans with booleans, nothing else is comparable. A bound of one type
    never matches values of another, and a mixed-type interval (`> 1 AND < 'x'`) is empty, as in the scan.
  * Numbers are widened to `f64` in the index, so the index range is inclusive and each candidate is re-checked with the exact `Value::compare` (an `i64` above 2^53 is still decided exactly).
  * Null, list and vector bounds skip the index and scan (WHERE decides), same rule as equality.
  * History is respected: the index holds every version of a node, and candidates are re-verified against the snapshot, so `AS OF` and updated or deleted nodes behave as in a scan.
  * `estimate()` treats an indexed range as "about a quarter of the label" for choosing which end of a path to start from. That is a fixed heuristic, not a statistic.
* **Directory lock.** `Database::open*` takes an exclusive advisory lock on `<dir>/LOCK` before reading anything and keeps it for the life of the handle. A second open, in the same process or
  another, fails with `Error::Invalid("... already open ...")`. Previously this was documented as "not enforced"; two processes appending to one WAL would interleave frames.
  Unix uses `flock` (released by the OS if the holder crashes, so it cannot go stale), Windows a no-share open of the lock file. If the filesystem cannot lock at all (some network filesystems), the open
  proceeds unlocked, as before.
* `examples/bench_range.rs`. Version 0.22.0.

## Verification
* `tests/range_index.rs` (3): a differential test runs ~800 generated queries (all four operators, flipped operands, two-sided and mixed-type intervals, `AND` with other predicates, an `OR` that must not be pushed
  down, `AS OF` before later updates and deletes) over 1,500 nodes holding ints, floats, strings, booleans, nulls and missing values, on a database with the index and one without, and requires identical
  results (more than 200 non-empty). A second test covers parameters, null bounds, empty intervals, i64 beyond f64 precision and hand-computed absolute answers; a third the `lookup_range` contract.
* `tests/dir_lock.rs` (3): same-process refusal then success after drop, refusal from a separate process, and no stale lock after a holder dies by `abort()`.
* Whole suite: `cargo test --release` (default features, rustc 1.97): 38 test binaries, 276 tests, 0 failures, 0 warnings. Built here with LTO off for speed; the PostgreSQL-backed tests run 0 tests without a database, as before.

## Measurements (`examples/bench_range.rs`, in memory, 200,000 nodes, ~20 matches per query, optimisation level 3, no LTO, 2 vCPU sandbox)
| | per query |
|---|---|
| label scan (no index) | 130,107 µs |
| range from index | 43 µs |

Counts were identical for all 50 queries. The ratio is large because the scan is linear in the label and the index cost follows the matches; a range that keeps most of the label gains little.

## Limits (stated plainly)
* One indexed key per node pattern is used; further range or equality conjuncts on other keys are only applied as filters.
* Range bounds must be literals or parameters. `a.k > b.k` and bounds computed from other variables are not pushed down.
* The Windows lock path was written to the documented `share_mode(0)` behaviour but **not compiled or run** here (Linux only). Treat it as untested.
* The lock is advisory: it protects against another nexgraph handle, not against a process that writes the files directly. It does not apply to the PostgreSQL backend, which has its own concurrency.
* Not changed: whole dataset in memory, single writer, no cost-based planner.
