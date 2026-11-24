# nexgraph: investor summary (v0.10)

*All figures below were measured in one Linux sandbox (single core, release build) on a 100,000-incident cyber-incident dataset. The dataset appears synthetic, so results show that the engine is correct and fast, not how it would perform on live customer data.*

## What it is
nexgraph is an embedded database written in Rust that stores security and business events as a **property hypergraph**: one event can link many entities at once (industry, country, attack vector, threat actor, stage, controls) without the workarounds a conventional graph database needs. It adds time-travel queries, a Cypher-style query language, durable storage (file log or PostgreSQL), TLS, role-based access with per-tenant isolation, spectral graph analytics, and Power BI export.

## What changed in this release
The release was driven by a real 100,500-row Enterprise Cyber Kill Chain file, which exposed problems a clean demo would not:

- **Data quality:** 500 duplicate incident rows silently inflated every total. The importer now de-duplicates by key or by exact row, and reports conflicts instead of merging them.
- **Answers a CISO asks for:** five new built-in analytics: kill-chain funnel, control effectiveness (with confidence intervals and stratified adjustment), financial exposure by segment, detection/response times, and defense-in-depth.
- **Statistical care:** the engine warns about a real pitfall. Adjusting for the stage an attack reached would hide MFA's benefit, because MFA works by stopping attacks early.

## What the data showed (association, not proof of cause)

| Control | Attack success with | without | Risk ratio (95% CI) |
|---|---|---|---|
| MFA | 26.8% | 37.4% | 0.72 (0.70-0.73) |
| EDR | 28.3% | 33.7% | 0.84 (0.82-0.86) |
| Security training | 28.1% | 32.4% | 0.87 (0.85-0.89) |
| IDS | 28.3% | 32.2% | 0.88 (0.86-0.90) |
| Firewall | 29.4% | 29.8% | 0.99 (0.95-1.02), no detectable effect |

- Mean time to detect: 97 min with EDR vs 143 min without; mean time to respond: 35 vs 63 min.
- Email is the riskiest vector (36% success vs ~25-27% elsewhere) and accounts for 39% of total recorded loss.
- Incidents reaching later kill-chain stages cost more: mean loss rises from about $30k (reconnaissance) to about $235k (impact).
- Success falls from 40% to 23% as controls in place rise from 2 to 5. The 0-1 control groups are small (49 and 805 incidents) and not monotone.

## Performance (100,000 incidents)
| Operation | Time |
|---|---|
| Import with de-duplication | 2.8 s |
| Any kill-chain analytic | 52-129 ms |

## Verification
Every kill-chain figure was cross-checked against independent computations in pandas, scipy and statsmodels (agreement to 1e-9 relative). The full automated suite passes.

## Honest limits
- Synthetic data; no customer deployment or real-world detection benchmark yet.
- In-memory engine, one writer, no replication; the PostgreSQL, TLS and multi-tenant paths were not re-tested this round for lack of a database server.
- Associations only; no causal claims.
- The code is largely AI-generated; disclosure rules should be checked before it is presented as unaided work.

## Suggested next steps
Pilot on real incident exports from one customer, add a streaming importer and replication, and benchmark detection quality on labeled live data.
