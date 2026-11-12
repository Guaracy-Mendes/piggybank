# Runbook: review and preview periodic accounting policies

Use this runbook to publish a synthetic calculation policy and reproduce its exact preview. These steps create immutable configuration and approval evidence. They do not enroll customer accounts or post fees/interest. Follow the [implementation specification](../implementation/periodic-accounting-policies.md) for rules and limits.

## 1. Prepare the running environment

For local evaluation follow [local development](local-development.md), including dependencies, migration, identity setup and approved seed. Current schema is 49; use matching binaries and migrations. Open the workforce portal at `http://127.0.0.1:4301`.

For a cloud evaluation installation, first complete the [GCP](../deployment/gcp/RUNBOOK.md) or [AWS](../deployment/aws/RUNBOOK.md) runbook and start its ordinary workstation tunnels. Policy previews require no provider administrative tunnel or signing key. Use a release containing migrations 48–49 and the periodic workspace; historical migration-47 archives do not contain this increment. Do not alter an already sealed release.

Expected result: API/worker health is ready, and current Finance maker/checker identities have separate canonical people, unexpired `finance_adjuster`/`finance_approver` grants and respective `finance-operations` REVIEW/APPROVE membership. Local seeded users are `reviewer` and `approver`; use the installation's credentials. Do not copy signing keys, cookies or tokens into terminal commands or tickets.

## 2. Choose the exact product and period

The seeded synthetic product UUID is `4f1a780b-3e31-48ef-a294-000000000001`, valid from 2026-01-01 through 2029-12-31. Verify the current product register if using a different fixture. A policy must fit that exact product version. Choose an unused rule code and version 1, or exactly the next version with a disjoint future interval.

The complete input files [monthly fee](../../contracts/examples/finance/periodic-monthly-fee.json) and [daily accrual](../../contracts/examples/finance/periodic-daily-accrual.json) contain reproducible synthetic examples. Their UUIDs identify examples; use fresh policy/calendar UUIDs when adapting API inputs to a new policy. The portal allocates and retains these UUIDs automatically for each exact request.

Expected result: monthly boundaries are first-of-month; a daily interval may begin on any covered date. The date range is at most 366 days. The chosen code/version does not compete with an existing publication.

## 3. Stage a monthly fee preview policy

Sign in as the Finance maker and open **Periodic policies**. Enter:

| Field | Example |
| --- | --- |
| Product version ID | Seeded UUID above |
| Rule code / version | `MONTHLY_MAINTENANCE_PREVIEW` / `1` |
| Effective from / exclusive until | `2026-11-01` / `2026-12-01` |
| Calculation kind | Monthly fixed fee |
| Monthly fee minor units | `250` (TST 2.50) |
| Calendar code | `SYNTHETIC_UTC` |
| Business weekdays | `1,2,3,4,5` |
| Holidays | Empty |
| Cutoff | `17:00:00` UTC |
| Reason | `Evaluate the synthetic calendar and exact fee while keeping charges disabled` |

The portal freezes coverage from policy start through exclusive policy end + 15 days. Use the API example for a different explicit covered calendar. Select **Save preview policy request**, retain the saved URL and review all returned material fields and the digest.

Expected result: a durable request appears as STAGED or PENDING after worker registration. Every policy states PREVIEW_ONLY. An uncertain response can be retried with the same material; the portal retains original request UUIDs/key in session storage. Do not clear this storage or change material merely because a response was lost.

## 4. Independently approve the exact material

Open the saved URL in a separate browser/session authenticated as the Finance checker. Compare product/version, effective range, fee/rate, day-count/rounding, full calendar coverage/weekdays/holidays/cutoff, reason, maker person, digest and expiry.

Select **I reviewed this policy, calendar and digest**, then **Approve preview policy**. The same canonical person cannot check their own request through another account/session. If the digest or material differs from what was reviewed, stop using that request and investigate the immutable source.

Expected result: APPROVED, distinct canonical maker/checker and a fixed expiry. The checker has no proposal or publication control. Revoked role/queue or expired authority prevents approval/publication.

## 5. Publish and recover acknowledgment

Return to the original maker's saved URL, select **Refresh policies**, then **Publish preview policy**. Read the returned original publication timestamp and digest. If the response is lost, refresh the saved URL before retrying; Core's immutable receipt makes accepted publication visible as PUBLISHED.

Expected result: exactly one immutable policy version, independent approval identity and unchanged customer balances/journals. Concurrent competing versions cannot both publish. A changed amount/calendar/key reuse is rejected, and a published interval cannot be overwritten.

## 6. Calculate the monthly example

In **Calculate an exact preview**, use period start `2026-11-01` and evaluation time `2026-12-01T17:00:00Z`. Select **Calculate preview**.

Expected result: period `[2026-11-01,2026-12-01)`, amount `250` exact minor units, fraction `250 / 1`, due `2026-12-01T17:00:00Z` and due-at-evaluated-time Yes. At `2026-12-01T16:59:59.999999Z` the due flag is No. A non-first-of-month start or a supplied balance for this monthly rule is rejected. The result is a computation, not a booking receipt.

## 7. Exercise a daily accrual and leap-day holiday rollover

Create another version-1 rule `DAILY_INTEREST_PREVIEW` for the same product, effective `2028-02-01` to exclusive `2028-03-01`, calculation **Daily credit accrual**, annual basis points `365`, UTC weekday calendar and holiday `2028-03-01`. Keep cutoff `17:00:00` and the explicit ACT/365 fixed, nonnegative closing booked, half-up-each-day semantics. Complete the separate maker/checker/publication steps.

For a preview use period start `2028-02-29`, operator closing booked minor units `9007199254740993`, and evaluation time `2028-03-02T17:00:00Z`.

Expected result: period `[2028-02-29,2028-03-01)`, holiday rolls due to `2028-03-02T17:00:00Z`, numerator `3287627727980462445`, denominator `3650000`, rounded amount `900719925474` and due Yes. Entering basis `5000` produces `1` minor unit; basis `0` produces `0`. Keep balances as decimal strings. Negative basis, unsupported currencies/scales or number-encoded money fail validation. UTC is the only supported timezone; DST-zone configuration is rejected.

## 8. Diagnose failures without changing accepted evidence

| Symptom | Action |
| --- | --- |
| STAGED persists | Check worker/readiness and original maker's current grants/REVIEW membership; registration cursor retries and wraps |
| CONTROL_RECORD_UNAVAILABLE | Preserve request ID/digest; investigate Controls availability or missing registered record |
| 403 | Recheck workforce session, CSRF/Origin, current role/queue and original canonical maker/checker |
| 409 | Reopen original scope; check digest/purpose/expiry, duplicate version, prior interval or publication winner |
| 422 | Check calendar coverage, unique ascending weekdays/holidays, UTC, exact cutoff, amount/rate semantics and product/period bounds |
| 503 | Restore the owned dependency and retry the original key/request |
| Old version | Read the saved original URL; version selection does not refresh or rewrite old receipts |

The worker recovers registration only; an operator still approves and the original maker publishes. Do not edit database policy rows, change old migration checksums, use preview results to charge customers or issue financial reversals from this workflow.

## Completion checklist

- [ ] Complete covered UTC calendar and exact material policy reviewed.
- [ ] Independent canonical checker and current grants/queues confirmed.
- [ ] Original request/key and immutable publication digest retained.
- [ ] Exact cutoff, amount and fraction reproduce the chosen example.
- [ ] Saved URL reopens on desktop/mobile; checker can inspect and calculate.
- [ ] No customer enrollment, journal, payment fee, accrual or GL booking was inferred from a preview.

For regression evidence run `make app-check`, `make test-unit`, `make test-integration`, `make test-payments`, `make test-customer`, `make test-staff`, `make test-ingress` and `make check`. Run shared-database integration and HTTP/browser suites sequentially. See [checkpoint evidence](../implementation/evidence/WP-24-periodic-policies.md) for current actual results.
