# nexgraph v0.13 (platform 0.13): what nine loan, insurance and customer datasets exposed

Datasets (your nine archives; `archive_2` is a byte-for-byte copy of `archive_3`):

| archive | content | what I did with it |
|---|---|---|
| 7 | `loan_data.csv`, 9,578 loans, **real LendingClub** data | scorecard, audit, PSI. Committed as a fixture |
| 6 | `loan_approval_dataset.csv`, 30,000 synthetic applications | leakage audit, disparity, conditional group gap, reason codes. A 3,000-row sample is a fixture |
| 8 | three tables: loan information (100,000), customer financials (50,000), historical defaults | three-table join through the graph, time split, PSI |
| 3 | insurance fraud graph: claimants, claims, providers, vehicles, 22 labelled rings | provider hotspots scored against the ring labels. `claims.csv` is a fixture |
| `archive` | customers, labels, behavioural log (489,663), **transaction ledger (1,033,406 rows)** | import scale / memory only (see Not done) |
| 1, 4, 5 | insurance claims (1,000), a 200-row health claims sample, two customer / premium sets | profiled only (shape, empty and duplicate columns). **Not modelled.** `insurance_claims.csv` has an entirely empty trailing column `_c39`; `fraud_reported` is 247 Y / 753 N |

## Findings and fixes

| # | Finding in v0.12 | Fix in v0.13 |
|---|---|---|
| 1 | **Tables could not be joined.** A8 keeps loan terms, customer financials and defaults in three files; the importer only builds hyperedges from one table, so a model could not use `Customer.Credit_Score` | `import_csv_nodes_keyed(db, label, csv, key_col, key_prop)` upserts rows onto existing nodes by a key (`ingest.rs`); features and conditions can now name `Label.prop` (`Customer.Credit_Score`, `Loan.Default=1`), read from a member node of that label. Edge properties and member-label names still win when a name is ambiguous |
| 2 | **A 1.03 M-row ledger needed 5.9 GB in one transaction** (the whole import is one atomic commit; the first attempt was killed on an 8 GB machine) | `HyperOpts.chunk_rows` / platform `chunk=` commits every N rows. Measured peak: 5.7 GB single, **3.7 GB with 100,000-row chunks** (same graph, 12 s vs 14 s). The remaining ~3.5 KB per row is the resident graph itself; chunking removes the transaction overhead, not that. Trade-off: a failure part-way leaves the earlier chunks committed. Platform bounds: 1,000 to 10,000,000 |
| 3 | No underwriting analytics at all | `uw.*` procedures (below), tenant-callable (allow-list gained `uw.`), four entries in the Analytics catalog (group "Underwriting") |
| 4 | Nothing detects a column that is the *answer* | `uw.audit`: AUC with a DeLong 95 % interval, information value and a strength label per feature. On A6 it flags `Approved_Amount` (AUC 0.000, i.e. perfectly inverted, IV 18.8), `Risk_Score` (AUC 0.925, IV 4.1) and `Default_Risk` (IV 3.1) as `leak?`; `Credit_Score` (IV 1.16, a strong honest predictor) is only `suspicious`. My first threshold flagged `Credit_Score` as a leak; I raised it (IV >= 2.0, or AUC >= 0.97 / <= 0.03) after seeing that |
| 5 | The 4/5 rule on raw rates misleads | `uw.disparity` reports it with chi-square, and `uw.group_gap` asks the follow-up: does the group fare differently from what a model **without** that attribute predicts. A6 `Marital_Status`, after conditioning on the other columns, the gaps are small but flagged for Married (observed rejection 40.96 % vs 42.30 % expected, z = -4.6) and Divorced (46.1 % vs 44.3 %, z = 2.8). Gender, Region and Channel show none. That is a screen, not proof of anything: the data is synthetic |
| 6 | `land.hotspots` also flags unusually *clean* groups, which is wrong for fraud | new optional 6th argument `side` = `both` (default, unchanged) / `high` / `low`. On the A3 claims, one-sided `high` flags 15 of 21 ring clinics, 17 of 22 ring repair shops and 6 of 11 ring attorneys, **with no provider outside a ring**; the default two-sided flag also reported 11 ring-free attorneys (low fraud rates) |
| 7 | `uw` CIs used 1.96 | uses 1.959963984540054 like the Wilson intervals (found by the reference test, difference 4e-7) |
| 8 | Platform: no way to import the new files | presets `loan_applications`, `lending_club`, `insurance_claims_graph`; `chunk=` parameter |
| 9 | A catalog RETURN of `distinct` does not parse (a reserved word, even backtick-quoted) | `uw.audit` omits that column from the catalog view (the procedure still yields it) |

## New procedures

| procedure | answers |
|---|---|
| `uw.audit(label, target, features[, filter])` | per feature: AUC (DeLong CI), information value, missing rate, `flags` (`leak?`, `suspicious`) |
| `uw.scorecard(label, target, features, idKey[, opts])` | L2-regularised logistic model: coefficients with SE, CI, p, odds ratio; train and test AUC (DeLong), Gini, KS, log-loss, Brier |
| `uw.gains`, `uw.scores` | decile table with Wilson intervals; per-record PD with three reason codes |
| `uw.disparity(label, groupKey, favourable[, filter[, minN]])` | adverse impact ratio per group, chi-square across groups |
| `uw.group_gap(label, target, features, idKey, groupKey[, opts])` | observed vs expected rate for a group from a model that excludes the group |
| `uw.psi(label, key, baseCond, compareCond[, bins])` | population stability index per bin and total |
| `uw.affordability(label, incomeKey, debtKey, loanKey, rateKey, termKey[, opts])` | recomputed DTI now / pro-forma / stressed (+200 bp default), over-limit shares, stored-vs-recomputed mismatch |

`opts` is `key=value;key=value` (`l2`, `test_pct`, `min_level`, `test`, `filter`, `limit`, `max_iter`, `set`). Split is FNV-1a 64 of the id modulo 100 (default 30 % test) or an out-of-time `test=<condition>`.

## Evidence

* `tests/uw.rs`, 10 tests. Replays `tests/fixtures/uw_expected.json`, generated by `uw_expected.py` with numpy, scipy, sklearn and statsmodels only: the scorecard (coefficients, SEs, p-values, odds ratios, train/test AUC with a brute-force DeLong, KS, log-loss, Brier) at **l2 = 1 against sklearn / a numpy Newton solver and l2 = 0 against statsmodels `Logit`**, audit AUC and CIs, PSI total and bin counts, disparity chi-square against `scipy.stats.chi2_contingency`, one-sided hotspots against scipy. Hand-worked affordability (payment 1010.87 stressed, 24.27 % pro-forma), the keyed-join semantics, chunked vs single import equivalence, out-of-time split, unknown-option errors.
* `uw::tests` unit tests (FNV vectors) and analytics catalog unit tests (every catalog entry builds from its own defaults; injection attempts in `opts` / features are refused).
* Platform `underwriting_through_the_api` (PostgreSQL): `chunk` bounds, the preset import with chunked commits, all four Underwriting analytics from their defaults, and the scorecard reference replayed through `/api/query`.
* Full suite results are at the end of this file.

## What the data says (descriptive, and mostly sobering)

* **A7 (real data):** 7-feature scorecard, test AUC **0.679** (95 % CI 0.653 to 0.705), Gini 0.358, KS 0.253 on 2,886 test loans; train 0.666. Significant: FICO (OR 0.78 per SD), inquiries, log income, small-business and home-improvement purposes. `dti` adds nothing once the others are in (p = 0.90) and `revol.util` is borderline (p = 0.08). That is a normal, modest credit model.
* **A8: there is no signal.** Across 100,000 loans, every one of 9 features has AUC between 0.496 and 0.503 and IV <= 0.001; the full scorecard's **test AUC is 0.498 (CI 0.489 to 0.507)**, also under an out-of-time split (0.497). Credit score distribution is stable between 2021 and mid-2023 (PSI 0.0007). The default flag is statistically independent of the features: the set is random noise, so *no model can be validated on it*. I did not hunt for a pattern; the engine reports the null honestly.
* **A6:** three columns are decision outputs (leakage, above); with them excluded, the reason codes for the highest-risk test records are previous default, late payments and credit score. I did not report an AUC for the cleaned A6 model.
* **A3 fraud graph:** all fraud (524 claims) sits on 22 rings that share a clinic, a repair shop and sometimes an attorney, but those providers also serve many legitimate claims (1,427 claims at ring clinics, 524 fraud). So `algo.components` merges everything into one component and is useless here; provider-level one-sided hotspots are the useful screen. I did not tune community detection.
* Taken together the sets are **synthetic** apart from A7. Results show correctness and behaviour, not credit or fraud conclusions.

## Not done / limits (read these)

* **No `cf.*` cash-flow / bank-statement module.** I scoped one for the transaction ledger (declared vs observed income, obligations, bounces, window dressing against its 150 labelled transactions) but did not build it. The ledger was only used to measure import scale. This is the main item left from this request.
* **No dedicated fraud-ring procedure.** Rings are screened with `land.hotspots(..., 'high')`; 6 of 11 ring attorneys and 15 / 17 of the clinics / repair shops are found, the rest are missed. No ring *membership* is recovered.
* The three-table join is available through the **library** (`import_csv_nodes_keyed`); there is no platform route for node imports, so through the API only single-table hyperedge imports work.
* A8's 100,000-row scorecard ran in about 0.5 s; the 1.03 M-row ledger needs ~3.7 GB resident even chunked.
* `uw.scorecard` is a plain logistic model, not a validated lending decision engine. No monotonic binning, no reject inference, no calibration beyond mean PD vs base rate, no regulatory validation (adverse-action wording, fair-lending law in your jurisdiction). `uw.disparity` and `uw.group_gap` are screens.
* Information value uses decile bins with 0.5 smoothing; it was not replayed against an independent implementation (AUC, PSI, chi-square, model fit and DeLong were).
* The shell `import hyper` still does not take `qualify`, `string_cols` or `chunk`; use the API or library.
* `frontend/dist` was not rebuilt; I did not check the new catalog entries in a browser.
* Compiled and tested with rustc 1.97 only. Code is largely AI-generated; review before production use.

## Suite result

Engine (`cargo test --release --no-fail-fast --features server`, PostgreSQL 16 for the PG tests) and platform (`cargo test --release`, `NGP_TEST_PG_URL` set): everything passes. Two things failed on the first engine run and are not product bugs: (1) a unit test whose expected chi-square tail values I had typed from memory (`chi2.sf(30, 5)` is 1.4748581e-5, not 1.4879e-5; the engine was right, the test now uses scipy values at 1e-9); (2) `tests/pg.rs` `mirror_detects_tampering...` hit the single-writer lock when run in parallel with another PG test binary; it passes with `--test-threads=1`, as does the rest of that file (16 / 16). Run the PG tests single-threaded.
