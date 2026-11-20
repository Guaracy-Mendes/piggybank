# Payment routing acceptance (WP-39)

`routing.py` drives customer payments through the real local issuer (native step-up), API, worker and provider
simulator on a fresh fixture database: `make test-routing`. Routing uses two interchangeable simulated providers
(`SYNTHETIC_RAIL`, `SYNTHETIC_RAIL_B`), following the user decision of 2026-11-18. Real providers and benefit against a
real baseline remain pending (DEC-03, DEC-04, DEC-07, DEC-11).

| Result key | What is proven |
| --- | --- |
| `nonexecution_fallback_same_identity_books_once` | Provider A rejects with `PROVIDER_CAPACITY_NOT_EXECUTED`. Core's route log shows A then B, and both providers received the identical instruction. B's signed settlement, delivered to the provider callback, books exactly one `EXTERNAL_PAYMENT`. |
| `stale_provider_repeat_has_no_effect` | Provider A replays its final (rejected) evidence after the re-route. The callback accepts it with no effect; the payment stays settled once. |
| `unknown_outcome_never_falls_back` | A lost response at A leads to `OUTCOME_UNKNOWN`, then inquiry finds it submitted at A. It is never sent to B and the route log shows only A. |
| `ordinary_rejection_final` | An ordinary rejection at A is declined with no fallback and no booking. |
| `policy_change_independent_and_shadowed` | The operator who proposed a health-weighted policy cannot approve it (403). A different approver applies it as a shadow, and the next payment records the shadow's choice. Activation before the minimum shadow decisions returns 409. |
| `ledger_invariants` | Every journal batch balances and every booked balance equals its entries. |

The database test in `crates/payments/tests/database.rs` adds a provider contradicting its own non-execution
(quarantined; Core refuses to book it), Core refusing a re-route on ordinary rejection evidence, policy-document
validation, a same-person alias, promotion after shadow evidence, rollback, and an in-flight instruction keeping its
original policy after rollback.
