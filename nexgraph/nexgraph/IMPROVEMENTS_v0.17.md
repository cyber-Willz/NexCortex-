# nexgraph v0.17: GAAP ledger

* New module `nexgraph::gaap` and procedures `gaap.trial_balance`, `gaap.income_statement`, `gaap.retained_earnings`, `gaap.balance_sheet`,
  `gaap.cash_flow`, `gaap.ledger`, `gaap.audit`. Write API: `Ledger` (`create_company`, `add_account`, `seed_reference_chart`, `add_period`,
  `post`, `reverse`, `close_period`, `set_cash_flow`, `set_account_active`). Full description in `GAAP.md`.
* `gaap.` added to the read-only procedure allow-list in `security`.
* Version 0.17.0 (the 0.16 archive still reported 0.15.0 in `Cargo.toml`).
* Tests: `tests/gaap.rs` (14), example `gaap_demo`. Verified against the reference PostgreSQL schema on PostgreSQL 16 (see `GAAP.md`).
