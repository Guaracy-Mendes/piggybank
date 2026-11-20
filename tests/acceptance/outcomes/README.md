# Outcome KPI acceptance (WP-41)

`outcomes.py` drives the real local issuer, API, worker and rail simulator on a fresh fixture database:
`make test-outcomes`. It onboards a customer, settles one external payment and has one rejected by the provider. It
then computes a KPI snapshot through the staff API. WP-41 is a local capacity template (user decision 2026-11-19).
Production outcomes, ratified targets and SLAs need DEC-07 and DEC-12.

| Result key | What is proven |
| --- | --- |
| `snapshot_labelled_local` | The snapshot has all eight KPIs and is labelled `LOCAL_SYNTHETIC_NOT_PRODUCTION_OUTCOMES`. |
| `onboarding_and_opening_counted` | The onboarding appears in KPI-01, as measured or as excluded while its 24-hour window is still open. KPI-03 counts the opened account as activated without remediation. |
| `payment_success_separates_provider_failures` | KPI-04 has denominator 2 and numerator 1 (the settled payment). The provider rejection is counted under `providerFailures`, not `bankFailures`. The SLA basis is stated as a local assumption pending DEC-04. |
| `integrity_zero_and_manual_kpi_not_claimed` | KPI-05 is zero. KPI-07 is `MANUAL_SAMPLE_REQUIRED`, not reported as met. |
| `snapshot_listed_with_digest` | A different staff member lists the snapshot with the same digest. |

The database test `crates/data/tests/cases/outcomes.rs` adds:
- role denial and window bounds;
- KPIs with no denominator, which are never reported as success;
- the risk-tier segment marked not recorded locally;
- immutability, and no direct table access for the Data role.
