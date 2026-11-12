# Periodic native basis operator runbook

This synthetic workflow captures ledger evidence and calculation previews. It does not enroll customers or post fees/interest. Read [basis semantics](../implementation/periodic-native-basis.md) before using a closed-period observation as evidence.

## Prerequisites

1. Follow [local development](local-development.md) through dependency startup, migrations, identity bootstrap, seed and app startup. Use the current schema including migration 0051. For a packaged VM, complete the [GCP](../deployment/gcp/RUNBOOK.md) or [AWS](../deployment/aws/RUNBOOK.md) runbook and workstation tunnels first.
2. Follow [periodic policy publication](periodic-accounting-policies.md) to create and independently approve/publish a daily or monthly `SYNTHETIC/PREVIEW_ONLY` policy. Retain the Finance request UUID and native policy UUID. Expected: state PUBLISHED with original Core publication digest.
3. Have a registered Core customer account with exactly that product version. Expected: the account is visible through the authenticated customer account workflow. A bare ledger UUID is insufficient.
4. Use distinct workforce `reviewer` (current maker and REVIEW queue) and `approver` (current checker and APPROVE queue). Synthetic local password: `Synthetic-Local-123!`; a packaged VM uses the per-install random password retrieved privately through its cloud runbook. Security/customer identities cannot inspect Finance artifacts.
5. For a meaningful open-day positive balance, use existing synthetic funding/payment workflows. Funding is a native financial command and is separate from capture; follow the [funding fixture instructions](finance-corrections.md) deliberately. Do not rewrite journal dates/amounts to make a historical basis.

## Capture and verify

1. Sign in at `http://127.0.0.1:4301`, open **Periodic policies**, select the published request and locate **Native ledger basis**.
2. Enter **Basis account ID** and **Basis period start**. Daily: an actual covered UTC date on or before today. Monthly: the first day of a covered calendar month. There is no balance or clock input.
3. Select **Capture or retry basis**. Expected: a saved basis URL, **Captured Core observation**, native policy/source/artifact digests, canonical person, original UTC capture/due clocks and source count.
4. Check status. Today/current month should be `OPEN_PERIOD_PROVISIONAL`. An ended period should be `CLOSED_PERIOD_OBSERVATION`; that label does not certify book close or absence of late facts.
5. Compare **Opening booked + Period change = Observed closing booked** with exact integer minor units. Review **Current booked/held/available at capture** separately. Current availability must equal booked minus held; it is not the historical closing basis.
6. Review **Calculation from captured facts**. It uses saved native evidence and clock. A calculation can be suppressed when the account registered after period start or daily closing basis is negative/out of range. Suppression must not be replaced with a typed operator balance. Even a displayed amount is a preview, not an amount owed.
7. Review **Native source facts**. Each entry includes ordinal, business date, signed delta, original posted UTC time, journal UUID, operation UUID and native fingerprint. Select **Load more basis facts** to inspect subsequent five-item pages. An empty closed prefix is valid; a nonzero current position does not backfill it.
8. Save the URL and reload it. Expected: identical original digest, count, totals and clocks. Sign in as approver and reopen the same URL: readable detail, no capture buttons. No Core journal, account version or hold changes result from capture/read.

## Replay and later postings

1. Keep the same account/date/policy fields and select **Capture or retry basis** again in the same maker browser session. Expected: original artifact returned, including after later financial postings.
2. To include new native facts, deliberately choose **Capture fresh observation**. Expected: new basis UUID/clock/digest; source membership reflects the new statement snapshot. Existing basis and source pages retain their original values.
3. Compare the two saved URLs. For late facts whose business date falls before an ended period's end, the fresh ended-period observation can differ. Neither capture establishes global finalization or permits another charge for the same economic period.
4. After an interrupted response, retry the original inputs in the same session before requesting a fresh capture. The UI's session-stored key protects that retry. A new browser session can read the saved basis URL or list; it does not recover an unknown session key automatically.

## Direct HTTP

Use the existing authenticated workforce session, CSRF token and exact permitted Origin. Do not print bearer/session credentials. The complete OpenAPI contract is [implemented.json](../../contracts/openapi/implemented.json).

```http
POST /api/v1/staff/finance/periodic-bases
Idempotency-Key: <original UUID retained for retry>
Content-Type: application/json
Origin: http://127.0.0.1:4301
X-CSRF-Token: <current workforce CSRF value>

{"policyId":"<native published UUID>","accountId":"<registered account UUID>","periodStart":"2026-11-11"}
```

Expected: 201 with `basis`, `digest`, nullable `calculation` and `calculationReason`. Use an actually covered date, not the illustrative date above.

```http
GET /api/v1/staff/finance/periodic-bases?policyId=<native UUID>&accountId=<account UUID>
GET /api/v1/staff/finance/periodic-bases/<basis UUID>
GET /api/v1/staff/finance/periodic-bases/<basis UUID>/items?after=0&limit=25
```

Expected: latest 100 exact-filtered verified records; original verified detail; stable items with whole-artifact digest and nullable `nextAfter`. Use `nextAfter` unchanged for the next page. Source count is bounded at capture, not by requesting fewer items.

## Troubleshooting and recovery

| Symptom | Operator action and expected result |
| --- | --- |
| 401/403 | Reauthenticate and confirm current canonical role/queue. A checker is read-only for capture. Do not reuse customer sessions. |
| 409 native guard or retry conflict | Check exact account/product/policy binding and covered period, then original actor/client/key scope. Reuse original material on retry; do not alter a saved artifact. |
| 422 | Check date/UUID syntax, nil identifiers, unknown fields and page bounds. Histories above 1,000 selected facts are unsupported and never partially stored. |
| Integrity conflict | Preserve original IDs and logs, investigate native/storage integrity. Do not rehash/repair artifacts manually or alter money. |
| Response lost after capture | Retry original key/material or reopen saved/listed basis; native evidence is authoritative. Audit relay recovers independently. |
| Audit not yet visible | Keep worker running. Its persisted 50-UUID cursor wraps past damaged records and retries; do not generate replacement financial activity. |
| Old basis excludes new facts | Expected immutable evidence. Explicitly capture a fresh observation with a new key. |
| Closed balance differs from current | Review business dates and source membership. Current booked balance must never replace historical evidence. |

Completion checklist: published exact policy; registered matching account; original basis UUID/URL/digest retained; all source pages reviewed as needed; historical/current totals distinguished; provisional/closed limitations understood; original replay verified; no financial posting or customer obligation claimed.
