# nexgraph v0.14 (platform 0.14): what the second batch of banking, AML, ATM and SEC filing data exposed

This builds on v0.13 (`IMPROVEMENTS_v0.13.md`, still valid). Your attachment this time was the 2010 SEC filing sets plus ten archives and one dictionary.

## What I tested, and what I did not

| data | result |
|---|---|
| `2010q1`, `2010q2`, `2010q4` (SEC Financial Statement Data Sets; `sub.txt`, `num.txt`, `pre.txt`, tab-separated, real filings) | imported (194,741 / 136,044 / 511,508 facts in 1.4 / 0.7 / 3.6 s) and checked with the new `fin.identity` |
| **`2010q3.zip` is corrupt**: `num.txt` fails to decompress ("invalid compressed data") | **not tested**; please re-send if you need it |
| `archive_10` AML_Suspicious_Transactions (100,000) | audit, hotspots, scorecard |
| `archive_8` FraudShield_Banking_Data (50,000) | import (duplicate ids), audit |
| `archive_9` financial_transactions (10,000), `archive_11` ATM (1,000) | audit, hotspots |
| `archive_7` financialreport.csv (181,229 rows) | **could not be read**: it is GBK-encoded (Chinese item names). Only the error message was improved. Not analysed |
| `archive_1` Forbes 2017 to 2026 (`.xlsx`) | **not tested**: the importer reads CSV only. The earlier TN land registry and `insurance_claims.csv` in that folder are the v0.12 / v0.13 data |
| `archive` Financial Statements.csv / `archive_3` accounting_data / `archive_4` financial_performance, audit_logs, pfm, big_data / `archive_5` short_financials (40,225 rows), insurance sets / `archive_6` financial_impact | profiled only (shape, no duplicate rows). Not analysed |
| the US Government Financial Report **data dictionary** | it describes tables (Statements of Net Cost, Balance Sheets, Social Insurance ...) but the data itself was not attached, so nothing to test |
| `archive_2`, parts of `archive_3`, `archive_6`, `archive_7`(loans), `archive_8`(loans) | repeats of datasets covered in v0.13 |

## Findings and fixes

| # | Finding in v0.13 | Fix in v0.14 |
|---|---|---|
| 1 | **Tab-separated files could not be imported at all.** Every SEC file is tab-separated | `HyperOpts.delimiter` (library) and platform `delimiter=comma|tab|semicolon|pipe`; an unknown value is a 400, and a tab file read as comma is refused for missing columns |
| 2 | Nothing could answer "do these statements add up?" on a long fact table | `fin.identity(label, entityKey, periodKey, tagKey, valueKey, totalTag, partTags[, tolPct[, blankKeys[, filter[, limit]]]])`. Per (entity, period): total vs sum of parts, relative gap against a tolerance. `?Part` marks an optional part. A group with a missing required part is **incomplete** and a group with two different values for a tag is **ambiguous**; neither is counted as a violation. `blankKeys` (`segments,coreg`) restricts to consolidated facts. Tenant-callable; one Analytics catalog entry (group "Financial statements") |
| 3 | A scorecard with a column that *is* the outcome printed an odds ratio of 33 billion and an AUC of 1.000 with no warning (AML data) | `uw.scorecard` now returns a `warning` row first when test AUC >= 0.995 or any standardised coefficient exceeds 8, telling you to run `uw.audit` |
| 4 | The upload error for non-UTF-8 files said only "must be UTF-8" | now says to convert (`iconv -f GBK -t UTF-8`, or `WINDOWS-1252`). No transcoding was added |

## What the data says

* **SEC 2010, Assets = Liabilities and Stockholders' Equity:** 962 of 962 (Q1), 1,016 of 1,016 (Q2), 2,949 of 2,949 (Q4) consolidated balance sheets match to within 0.01 %. Zero violations. The filings are internally consistent on that identity.
* **SEC 2010 Q1, Assets = Liabilities + StockholdersEquity:** only 501 of 972 balance sheets even report both parts (471 are `incomplete`: many filers never tag a Liabilities total). Of the 501, **118 are off by more than 1 %**; allowing minority interest and redeemable / temporary equity as optional parts cuts that to **21**. So the "violations" are almost all a modelling choice (what equity includes), not errors: the worst remaining gap is 6.2 %. Do not read 118 as 118 misstatements.
* **AML data:** `Risk_Score` separates perfectly (AUC 1.000, IV 12.7): it is derived from the label, not a predictor. Real structure: five countries (North Korea, Afghanistan, Myanmar, Iran, Syria) have suspicious rates of 22.7 % to 23.5 % against 9.4 % overall, while Malaysia sits at 1.0 %. `Customer_Risk_Profile` carries signal too. `Amount` is slightly *lower* in suspicious cases (AUC 0.472) and `Annual_Income` and `Transaction_Type` are noise. It reads as a rule-based synthetic set, not observed behaviour.
* **FraudShield:** `Transaction_ID` is not unique: 1,361 repeated ids among 50,000 rows (about what random six-digit ids would give), and 20 rows lack an id, customer or merchant. With key-dedup the importer keeps the first record per id, drops 1,359 *different* records and says so in `rows_skipped`. Fraud rate is 4.8 % (2,423). Of 13 features, only `Unusual_Time_Transaction` and `Is_International_Transaction` carry any signal (IV about 0.08, weak); the other eleven have AUC 0.48 to 0.51 and IV <= 0.008. I did not fit a scorecard on it.
* **financial_transactions (10,000):** every feature has IV <= 0.010 and AUCs whose intervals include 0.5, including `Known Threat Flag` and `Login Location Match`. No usable signal for the suspicious flag.
* **ATM (1,000):** failure rate 4.3 %; no city differs. One ATM (1007: 3 failures in 12) is flagged one-sided at the Bonferroni level; with only 12 transactions the normal approximation behind that flag is unreliable (a known limit of `land.hotspots`), so treat it as a lead at most.
* All of these are synthetic except the SEC filings.

## Evidence

* `tests/fin.rs`, 4 tests. `real_sec_balance_identities_match_pandas` imports a fixture of the real 2010 Q1 balance-sheet facts (6,970 rows, tab-separated) and replays `fin_expected.json` from `fin_expected.py` (pandas only): groups, checked, violations, incomplete and the worst relative gaps for three identity definitions. Hand-built cases cover tolerance, optional parts, incomplete, ambiguous, blank keys and argument errors. Another test covers the separation warning (and that an honest weak model gets none).
* The same logic was checked by hand against pandas on the full Q1 file: 501 / 118 / 21 and 962 / 0, the same numbers `fin.identity` returns.
* Platform: unit tests for `fin.identity` argument compilation and injection attempts; the API test now also imports the SEC fixture with `delimiter=tab` (and refuses `delimiter=semi` and the comma read) and runs `fin.identity` through the API (962 checked).
* Full suite results are at the end of this file.

## Not done / limits

* **No Excel import.** The Forbes workbooks, and the xlsx copies of other sets, need converting to CSV first.
* **No transcoding.** Non-UTF-8 files (GBK, Latin-1) must be converted before upload; the 181,229-row `financialreport.csv` was not analysed.
* `fin.identity` checks one equation at a time over a long table. It does not know XBRL calculation linkbases, signs (`negating`) or units beyond what you filter; the filter is a single condition (for example `uom=USD`), not a conjunction.
* `fin.identity` was run on balance-sheet facts only. Income-statement and cash-flow identities, and the `pre.txt` presentation files, were not used.
* Q3 is untested (corrupt upload); Q1, Q2 and Q4 were imported and checked, but only Q1 is a committed fixture and replayed against pandas.
* The remaining profiled-only files were not modelled. The US Financial Report needs its data files.
* `frontend/dist` was not rebuilt. Compiled and tested with rustc 1.97 only. Code is largely AI-generated; review before production use.

## Suite result

Engine (`cargo test --release --no-fail-fast --features server -- --test-threads=1`, PostgreSQL 16 for the PG tests) and platform (`cargo test --release -- --test-threads=1`, `NGP_TEST_PG_URL` set): every test passes, including the new `tests/fin.rs` (4) and the extended platform `underwriting_through_the_api`. Run the PG-backed tests single-threaded, as noted in v0.13.
