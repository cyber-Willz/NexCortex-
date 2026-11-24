# nexgraph v0.11 (platform 0.11): improvements found by running the ALM insurance dataset v2.1

Dataset: `alm_insurance_dataset` v2.1 (synthetic life and annuity ALM data, modelling date 2023-04-29): 3,501 holdings, 77,704 asset
cash flows, 47,880 liability cash-flow rows (traditional book), a monthly cash-flow gap table (unit-linked outflows), key-rate durations,
and 9,700 stochastic Hull-White paths x 121 months (1.17M rows). Until now the platform could *load* it (16/16 tables, per the dataset's own
integration report) but had no ALM analytics: every ALM question needed an external pandas script.

## What the dataset exposed

| # | Finding in v0.10 | Fix in v0.11 |
|---|---|---|
| 1 | No asset-liability procedures: funding ratio, duration gap, rate shocks, cash-flow matching, key-rate gaps and path VaR were not expressible in NQL | New `alm.*` procedures (`src/alm.rs`), tenant-callable, and seven entries in the platform's Analytics catalog (group "ALM") |
| 2 | `killchain.exposure` / `defense_depth` accepted a **continuous, mostly-zero column** as a 0/1 outcome (reported in the dataset's integration report) | Outcomes and controls with more than 1 % numeric values other than 0 and 1 are refused with the count (`require_binary`) |
| 3 | Importing each ALM table needed a hand-written `members=`, `time=`, `dedup=`, `props=true` query (and `props` defaulted to *false*, which silently left every numeric column unreadable by analytics) | Ten named presets: `preset=alm_assets`, `alm_asset_cashflows`, `alm_liability_cashflows`, `alm_cashflow_gap`, `alm_key_rate_durations`, `alm_scenarios_stochastic`, `alm_metrics`, `alm_policies`, `alm_policies_unit_linked` (+ `kill_chain`). A preset keeps member columns as event properties; the error for an unknown preset lists them all |
| 4 | Two liability books that do not reconcile (the dataset documents that its assets back the unit-linked book, not the traditional one: funding ratio 0.26) | `alm.duration_gap` and `alm.rate_shock` return a `warning` when the funding ratio is below 0.5 or above 2, and every procedure takes a `Key=Value` filter so a book can be chosen explicitly |
| 5 | 93 bond cash flows fall in month 0 (paid days after the valuation date) with discount factors of about 0.9994 | Month-0 flows keep their stated discount factor and take no shift, so repricing at zero shift equals PV exactly. (Found when the engine disagreed with the reference calculation by 2e-6.) |

## New procedures

| procedure | answers |
|---|---|
| `alm.position(label, mvKey, durKey[, convKey[, groupKey[, filter]]])` | market value, MV-weighted duration and convexity, DV01, share, largest holding and Herfindahl index by segment |
| `alm.liability_profile(label, cfKey, dfKey, monthKey[, groupKey[, filter]])` | PV, modified duration, convexity, DV01, implied yield, half-life and PV share within 5 / 10 years |
| `alm.duration_gap(aLabel, mv, dur, conv, lLabel, cf, df, month[, aFilter[, lFilter]])` | funding ratio, surplus, leverage-adjusted duration gap, net DV01, net dollar convexity |
| `alm.rate_shock(.. , shocksBp[, aFilter[, lFilter[, bondLabel, bondCfKeys, bondDfKey, bondMonthKey[, bondFilter]]]])` | surplus and funding ratio under parallel yield shifts; liabilities exact, assets by duration+convexity or exact from bond cash flows |
| `alm.cashflow_ladder(inLabel, inMonth, inKeys, outLabel, outMonth, outKey[, bucketMonths[, inFilter[, outFilter]]])` | inflow, outflow, cumulative gap, cover ratio, shortfall flags by bucket |
| `alm.krd_gap(label, sideKey, pvKey, krdKeys[, assetSide[, liabSide[, filter]]])` | DV01 of assets vs liabilities at each key tenor |
| `alm.scenario_risk(label, scenarioKey, monthKey, valueKey, horizon[, baseMonth[, mode[, exposure[, exposure2[, alpha]]]]])` | distribution of the change between two months across paths; with an exposure, P&L mean, VaR, CTE and probability of loss |

Conventions: annual-effective yields; `y = df^(-1/t) - 1` per cash flow; a shift `d` reprices `cf (1+y+d)^(-t)`; modified duration
`sum(t pv/(1+y))/PV`; DV01 = loss per +1 bp. Liabilities are outflow-positive. A segment with non-positive PV (term and whole life, where premiums
exceed claims) has NULL duration and convexity but a signed DV01. The dataset's own `mod_duration` and `convexity` columns agree with the values
implied by its asset cash flows (median relative difference 1e-4), which is why this convention was chosen.

Example (assets against the unit-linked book, then a +-100 bp shock repriced exactly, then 5-year rate VaR):

```
CALL alm.duration_gap('ASSET','market_value','mod_duration','convexity','GAPM','unit_linked_outflow','discount_factor','month') YIELD *
CALL alm.rate_shock('ASSET','market_value','mod_duration','convexity','GAPM','unit_linked_outflow','discount_factor','month',[-100,100],null,null,
                    'ASSETCF',['coupon_or_dividend_cf','principal_cf'],'discount_factor','month','asset_class!=EQUITY') YIELD *
CALL alm.scenario_risk('PATH','scenario_id','month','short_rate',60,0,'change', <-net_dv01*10000>, <net_dollar_convexity>, 0.995) YIELD *
```

## Evidence

* `tests/alm.rs`, 10 always-on tests on small tables whose expected numbers were computed independently in Python (to 1e-9 relative), plus
  `real_dataset`, which loads the real CSVs and replays **98 cases** generated by `tests/fixtures/alm_expected.py` with pandas/numpy
  (position by class and rating, liability profile by product, duration gap on both books, rate shocks exact and approximate, 58-bucket
  ladders, key-rate DV01, path distributions and VaR/CTE). All 98 match to 1e-9 relative.
  Run: `python3 tests/fixtures/alm_expected.py DATA_ROOT exp.json && NG_ALM_DATA=DATA_ROOT NG_ALM_EXPECTED=exp.json cargo test --release --test alm`.
* The traditional-book PV (10,210,620,000) reproduces the dataset's `books_reconciliation.csv` figure; bond PV from cash flows equals bond market
  value to 2e-6.
* Full stochastic file (`examples/alm_full_paths.rs`): 1,173,700 events imported in 4.4 s (RSS 2.9 GB); `alm.scenario_risk` over all 9,700 paths
  takes 0.32-0.43 s per call, and matches pandas (mean, 0.5 % and 99.5 % quantiles, VaR).
* Platform: unit tests for argument compilation (shock lists, optional exact-repricing keys all-or-none, numeric literals, filter injection), and `alm_dataset_through_the_api` (needs PostgreSQL, `NGP_TEST_ALM_DATA`, `NGP_TEST_ALM_EXPECTED`): the six presets import with exact row counts, all 7 ALM analytics run from catalog defaults, and the same 98 cases match through `/api/query`.
* Full engine suite (`cargo test --release --no-fail-fast`): 19 test targets, 0 failures. Two fixes were needed: a stale v0.10 test that expected date-only timestamps to be invalid (they have parsed as midnight UTC since v0.9), and the `require_binary` message, which keeps its original wording.

## Findings about the data (descriptive)

* Assets of $2.68B against the traditional book's $10.21B PV: funding ratio 0.26 (the guard fires). Against the unit-linked outflows in
  `cashflow_gap_monthly` ($1.48B PV): 1.81.
* The PV of `unit_linked_outflow x discount_factor` is $1,482.3M, 0.51 % below the $1,489.9M in `books_reconciliation.csv` (month-end discounting
  versus OSEM's dates). The dataset documents the larger figure; neither is wrong, but they are not the same number.
* Duration+convexity versus exact bond repricing: -0.53 % / +0.55 % of the bond P&L at -100 / +100 bp, -2.07 % / +2.27 % at +-200 bp,
  -4.52 % / +5.21 % at +-300 bp. The shortcut understates both large gains and large losses.
* TERM20 (-$169M) and WL (-$74M) have negative net-outflow PV: they have no duration.

## Limits

* Parallel shifts of an annual-effective zero curve only; key-rate or twist shocks need the `krd_gap` DV01s, which are first-order.
* `scenario_risk` applies a second-order exposure you supply to a one-variable change. It is not a full stochastic revaluation of the balance sheet
  (no path-dependent lapses or crediting, no equity or spread leg unless you run it per variable and combine them yourself).
* Assets without cash flows (equities, cash) have zero rate sensitivity. Credit spread and equity risk are not modelled by `rate_shock`.
* The synthetic dataset: results validate correctness and scale, not real-world ALM conclusions.
* Compiled and tested with rustc 1.89 only (`rust-version = "1.85"` unverified). `frontend/dist` was not rebuilt; the UI reads the catalog, so the new analytics should appear, but this was not checked in a browser.
  unless stated in the platform notes below.
* Code is largely AI-generated; review before production use.
