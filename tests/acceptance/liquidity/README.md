# Liquidity forecasting acceptance (WP-40)

`liquidity.py` drives the real local issuer, API, worker, rail simulator and liquidity simulator on a fresh fixture
database: `make test-liquidity`. A fresh runtime has no warehouse history, so the test seeds 21 business days of
synthetic settlement outflow facts directly into `analytics.facts`. Real treasury feeds and limits remain pending
(DEC-03, DEC-06, DEC-07, DEC-12).

| Result key | What is proven |
| --- | --- |
| `missing_evidence_degraded_and_escalated` | With no signed liquidity evidence the forecast is `DEGRADED`, shows no confirmed funds, and opens an owned `FINANCE` case in `finance-operations`. Nothing is restricted and no action is proposed. |
| `confirmed_breach_restricts_and_proposes` | Fresh evidence below the buffer gives a `CONFIRMED` `BREACH`. It records a restriction and proposes one settlement-prefunding action. |
| `breach_stops_external_submission` | A customer payment confirmed during the restriction waits as `RAIL_PAUSED` / `LIQUIDITY_RESTRICTED`, with no Core hold and no provider instruction. |
| `automated_action_independent_and_booked_once` | The maker cannot approve (403). The maker confirms, and a different approver approves. Core books one `LIQUIDITY_PREFUNDING` batch, and a retry returns the same batch. |
| `independent_lift_resumes_payment` | A lift is refused while the latest forecast still breaches (409). After fresh evidence gives a within-limits forecast, the maker still cannot apply the lift (403); the approver can. The waiting payment is then submitted to the provider. |
| `forecast_reproduces` | Reproduction of the breach forecast matches its digest. |
| `scheduled_forecast_by_worker` | A policy version with `scheduleMinutes: 1`, proposed by the maker and applied by the approver, makes the worker record a forecast with no person attached. |
| `ledger_invariants` | Every journal batch balances and every booked balance equals its entries. |

The database test `crates/finance/tests/cases/liquidity.rs` adds:
- escalation reuse within an episode;
- reproduction after late facts;
- a same-person alias with both Finance roles;
- tampered or swapped Core commands;
- a stale feed superseding a newer proposal;
- invalid policy documents;
- a capped action under a lower limit;
- immutability and the Finance role's lack of write access.
