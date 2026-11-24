# nexgraph v0.10: improvements found by running the Enterprise Cyber Kill Chain dataset

Dataset: `Enterprise_Cyber_Kill_Chain_Dataset.csv`, 100,500 rows x 50 columns, one incident per row (industry, country, company size,
attack vector, threat actor, kill-chain stage, five binary controls, outcome flag, loss, detection/response times, risk scores).
It looks generated (Threat_Actor fully determines Attack_Complexity; Industry fully determines Compliance), so these results validate
correctness and scale, not real-world detection quality.

## What the dataset exposed

| # | Finding in v0.9 | Fix in v0.10 |
|---|---|---|
| 1 | **500 exact duplicate rows** (same `Incident_ID`, identical content). `import_csv_hyperedges` loaded all 100,500, so every count/sum was inflated (loss records 95,471 instead of 94,994; mean loss 75,515.67 instead of 75,562.62) | `import_csv_hyperedges_with(.., &HyperOpts { dedup })`: `Dedup::ExactRow` (128-bit row fingerprint) or `Dedup::Key(col)`. A repeated key with *different* content is counted in `key_conflicts` and skipped (first wins), never merged silently. `ImportReport` gains `duplicates_skipped`, `key_conflicts`. Default behaviour unchanged |
| 2 | Member columns (vector, stage, ...) are not edge properties, so `WHERE e.Attack_Vector = 'Email'` matched nothing and there was no edge-only way to group | `HyperOpts.keep_member_props` stores them on the edge. Shell: `.import hyper LABEL file.csv name COL:Label,.. - dedup=key:Incident_ID props` |
| 3 | Hyperedge-per-row data forces `MATCH (a)-[e]-(b)` plus `count(DISTINCT e)`; plain `count(e)`/`sum(e.x)` multiply by the number of member pairs | New `killchain.*` procedures aggregate events directly (below) |
| 4 | No way to ask "does MFA reduce attacks?" with uncertainty | `killchain.control_effect`: risk ratio, 95% CI, p-value, per-stratum ratios and Mantel-Haenszel pooling |
| 5 | The data holds a trap: MFA lowers success 37.4% -> 26.8% (RR 0.717) but within each *attack stage* the effect vanishes (MH RR 0.967), because MFA works by stopping attacks early (stage is a mediator). Stratified by vector (a pre-treatment variable) RR stays 0.717 | Documented in the module header and tested (`mh_stage_mfa_rr`) |

## New procedures (`src/killchain.rs`)

`killchain.funnel(label, stageKey, outcomeKey[, order[, lossKey[, detectKey]]])`,
`killchain.control_effect(label, controlKey, outcomeKey[, groupKey])`,
`killchain.exposure(label, groupKey, outcomeKey, lossKey[, minN])`,
`killchain.response(label, groupKey, detectKey, respondKey)`,
`killchain.defense_depth(label, [controlKeys], outcomeKey[, lossKey])`.

A key is read as an edge property first, else as the name of the member node with that *label* (so `'Vector'` works without
`keep_member_props`, `'Attack_Vector'` needs it). A key present on no event raises an error listing the event's properties instead of returning nothing.
Missing values are excluded and counted, never treated as zero. Rates carry Wilson 95% intervals; ratios carry log-normal intervals.

## Evidence

* `tests/v010.rs`, 11 tests. Ten run anywhere on a 10-row table whose expected values come from pandas, scipy and statsmodels
  (`Table2x2`, `StratifiedTable.riskratio_pooled`, `proportion_confint(method='wilson')`, pooled z-test). The eleventh
  (`NG_KC_CSV`, `NG_KC_EXPECTED`) replays funnel, five control effects, MH by vector and by stage, exposure by vector, response by EDR and
  defense depth on the real file and matches the independent computation to 1e-9 relative (p-values compared on the log scale; they reach 1e-220).
* Two unit tests check the normal tail against scipy (to 1e-9 relative, out to z = 10) and the Wilson interval.
* Full `cargo test --release` (rustc 1.100): all pass; PostgreSQL, TLS and server tests skip without a server, as before.
* Timing (one core): import 2.0 s (no dedup) / 2.8 s (dedup + member props), 100,000 incidents, 44 member nodes;
  every `killchain.*` call over 100,000 incidents takes 52-129 ms.

## Limits

* Only rustc 1.100 was compiled; `rust-version = "1.85"` in Cargo.toml is unverified.
* Associations only: no causal claim. Observational stratification handles only the variables you stratify by.
* `defense_depth` is thin at the low end (49 incidents with zero controls) and non-monotone there (34.7%, 43.5%, 40.2%, 34.0%, 29.0%, 23.2% for 0..5 controls); read the intervals.
* The Firewall control shows no detectable effect (RR 0.987, 95% CI 0.953-1.022, p = 0.45).
* `RETURN *` is still unsupported (use `YIELD *` then named columns). Very small p-values print as long decimals.
* Dedup holds one fingerprint per row in memory; no streaming import.
* The PostgreSQL backend, TLS and multi-tenant security were not exercised in this round (no server in the sandbox).
* Code is largely AI-generated; check disclosure rules before presenting it as unaided work.

## Platform release (this archive also contains `../platform`)

Engine changes made for the web platform (see `../platform/README.md`):

* `Session::import_hyperedges_csv`: bulk import through a tenant session. Needs `writer`, enforces the entity quota,
  audits before writing (fail closed), refuses platform admins (no tenant data).
* `ControlPlane::list_tenants` (platform admin) and `list_users` (tenant admin of that tenant, or platform admin);
  password hashes are never selected.
* Tenants may now call `killchain.*` and `db.stats` (the procedure allowlist previously rejected both).

## v0.10.1: second dataset (Federated Cyber Threat Intelligence, 15,000 events)

Imported through the platform API (10 organizations, 100 agents; CRLF line endings; no duplicates or nulls): 15,000 events,
137 entities, 4.9 s. Describe, cross-tabs, time series, neighbourhoods and response times matched pandas. Two defects found and fixed:

1. **Silent wrong answer.** `killchain.exposure` accepted a continuous 0-1 column as the outcome and reported a "success rate"
   computed from the 7% of rows that happened to equal exactly 0 or 1 (1,112 of 15,000), beside `incidents = 15000`; a categorical
   outcome returned NULL. Now an outcome or control must be binary for at least half the events or the call fails with the count,
   and `exposure` / `funnel` report `outcome_known`.
2. **No way to express "attack vs Normal".** Outcomes and controls may now be tests: `Category!=Normal`, `Severity=Critical`,
   `Protocol=HTTPS` (also inside the `defense_depth` control list). Validated against pandas / statsmodels on this dataset
   (exposure by organization with Wilson intervals, crude and Mantel-Haenszel risk ratios, funnel by severity, defense depth).

Findings are descriptive: 69-72% of events per organization are attacks (differences are small), HTTPS has no detectable
association with attack (RR 1.004, 95% CI 0.978-1.031), and every Medium-or-higher severity row except 14% of Medium is an attack,
i.e. severity is nearly a label for the category rather than an independent signal. The data look generated.
`platform/api/tests/federated_check.py` repeats these checks against a running server.

## v0.10.2: third dataset (Cyber Threat Intelligence flows, 15,000 rows x 37 columns)

Network-flow records (destination port, protocol, connection state, attack category, 33 numeric traffic features; no timestamp,
no duplicates, no nulls). Imported through the platform API with four member columns and `props=true`: 15,000 events, 25 entities.
Describe on six features (including heavy-tailed `Flow_Bytes_per_sec`), the Port x Category cross-tab, exposure by protocol,
crude and Mantel-Haenszel risk ratios, response statistics and a funnel over connection state all match pandas / statsmodels.

Two defects found and fixed in the platform API:

1. **Numeric entity names could not be explored.** `GET /api/graph/neighborhood?label=Port&name=22` returned 404 because the
   name was sent as text while the node stores the integer 22. The lookup now tries the text, then the number it spells.
   All 10 ports match pandas; unknown and odd names still return 404.
2. **A time series over a missing timestamp key returned an empty 200.** The API now answers 400 and says to use a column
   imported with the time option. (The engine keeps its documented v0.9 behaviour of an empty series.)

Known limit, not fixed: a numeric column that was not imported as a time column is still accepted by `stats.timeseries`
and bucketed as if it were epoch seconds (`Flow_Duration` produced 6 meaningless buckets). The engine cannot tell the two apart.

Findings are descriptive and the data look generated: attack share is 66-67% in every connection state, the same in every
protocol, so none of the four categorical columns separates attacks from normal traffic.
`platform/api/tests/flow_check.py` repeats the checks against a running server.

## v0.10.3: four anti-money-laundering datasets

| Dataset | Size | Import (platform API, PostgreSQL-backed) |
|---|---|---|
| AML Suspicious Transactions | 100,000 rows, microsecond timestamps | 11 s, 5,022 entities |
| ML.csv + MLtag.csv (two files, shared account ids) | 2,340 + 1,484 rows | 0.6 s + 0.2 s |
| Synthetic AML network (accounts, 42,175 transactions, 22 ground-truth rings) | 3,500 + 42,175 rows | 0.4 s + 4.7 s |
| OSINT crypto graph (transactions, wallets, alerts, bridge/mixer events) | 150,000 + 30,000 + 15,000 + 10,000 rows | 24 s + 4.5 s + 1.9 s + 0.9 s; 30,658 entities, 205,000 events |

Verified against pandas / statsmodels: exposure with Wilson intervals and loss percentiles by country, payment method, network,
typology; crude and Mantel-Haenszel risk ratios (High-risk profile vs suspicious: RR 19.6; USDT vs laundering pattern);
describe; cross-tabs; microsecond timestamps (time-series totals exact); duplicate and conflicting-key accounting across files
(3 exact repeats, 39 conflicts, as pandas counts them); and the cross-file graph joins: one wallet's neighbourhood shows its
transfers, wallet record and 9 alerts, and each of the 22 laundering rings' graph neighbours equals `member_accounts` in the
ground-truth file exactly. The two files that share account ids reuse the same nodes (no duplicate accounts).

Defects found and fixed:

1. **Spectral clustering could not be stopped.** On a 5,022-node graph `graph.spectral_clusters` ran past the 60 s limit, the client got
   504, and the computation kept consuming a worker. It uses a dense eigen-decomposition, so cost grows roughly with the cube of the node
   count. The API now refuses it above `NGP_SPECTRAL_MAX_NODES` (default 1000) with a clear message. The default is a conservative guess:
   timings here came from a debug build (501 nodes took 17.5 s; 1,178 timed out), release timings were not measured.
2. **Boolean entity names could not be explored** (`Mule=True` returned 404, the same cause as numeric ports in v0.10.2); now handled.
3. **Category values with real-world punctuation were rejected**: `Mixer Deposit (Tornado)`, `Rapid Layering Fan-Out (>8 hops in 1h)`,
   `Mule / Layering Account`. Values may now contain anything except quotes, backslash, backtick, semicolon, `=` and control characters;
   property names stay strict. Injection strings are still rejected (unit-tested).
4. **PageRank ranked category hubs instead of wallets** (`Direct Peer-to-Peer`, `Ethereum Mainnet` at the top). `graph.pagerank` and
   `graph.communities` take an optional entity type.
5. Imports now have their own time limit (`NGP_IMPORT_TIMEOUT_SECS`, default 900) instead of the 60 s request limit, because a
   client that sees a 504 would retry an import that is still running and load it twice.

Not changed: the `total_volume_usd` in `laundering_rings.csv` differs from the sum of the ring transactions by up to $682,390 (a property
of the dataset); PageRank on the synthetic network ranks only 3 of the 132 mules in its top 50 accounts, so centrality alone does not
find the rings (neighbourhood joins do). All four datasets are synthetic. The parquet copies were not compared with the CSVs.
Scripts: `platform/api/tests/dataset_checks/` (need a running server and tenant names that exist only in the test sandbox).

## v0.10.4: thirteen supply-chain, logistics, retail and food-delivery datasets

About 60 tables (one spreadsheet, 13 relational tables of a synthetic supply chain, plus 20-odd single files; 2 archives were
repeats of earlier files) were run through an automated differential harness (`platform/api/tests/dataset_checks/sweep.py`): per
file it chooses member columns, key, time column and an outcome, imports through the API, then compares node and event counts,
`stats.describe` on numeric columns, a cross-tab, `killchain.exposure` (Wilson intervals, loss totals and p90), a risk ratio against
statsmodels and the time series against pandas. 52 files imported and matched; the largest were Food Delivery (17 MB), supply-chain
history (15 MB, 15 s), Logistics routes (15,000 rows with a UTF-8 BOM) and Superstore (9,994 rows, Windows-1252).

Defects found and fixed:

1. **Bare dates were rejected** (`2025-10-16`): only `YYYY-MM-DD HH:MM` and `T` forms parsed, so any file with plain dates imported nothing.
   Date-only is now midnight UTC. The parser also accepted impossible days (`2025-02-30` quietly became March 2); it now validates
   month length and leap years.
2. **An import that created nothing answered 200.** It now answers 400, "nothing was imported: N rows read, N skipped; first problem: ...".
3. **US and European dates** (`11/8/2016`, `9/22/2025 1:30 PM`) cannot be told apart, so they are declared, never guessed:
   `date_format=mdy|dmy` on the API (a selector on the Import page), `dates=mdy|dmy` in the shell, `HyperOpts.date_order` in Rust.
   Superstore and Inventory_Transactions import and match pandas with `mdy`.
4. **Column names with symbols were rejected** by analytics (`moisture_content_%`, `Profit ($)`, `Qty (kg)`); property keys now take any
   printable text except quotes, backslash, backtick, semicolon and `=`. Import-time labels must be identifiers (a clear 400 otherwise),
   since a label becomes `:Label` in a graph pattern; the column name itself may be anything without a comma or colon.
5. **The browser could silently corrupt a non-UTF-8 file** (`File.text()` substitutes U+FFFD). The Import page now refuses such a file
   with instructions; the API already refused non-UTF-8 bodies.

Findings that are not defects:
* `WEATHER_CLEAN.csv` has another table (ABC segments) appended from line 3287; the importer loaded the 3,285 valid rows and reported the
  268 it skipped with the first offending line, as designed.
* A missing value in an outcome column makes that event's outcome unknown (`outcome_known` shows it) rather than "not the value":
  one channel with an empty ship mode is excluded from `ship_mode != Air` rates.
* A single-entity time series needs two entity columns (an event links at least two entities); `logistics_data_5k.csv` was imported
  with the tier and episode id as members.
* UTF-8 BOM files (Excel exports) import correctly. Spreadsheets (`.xlsx`) must be exported to CSV first; the sheet was converted
  with pandas for the test. The parquet copies were not compared with the CSVs.
