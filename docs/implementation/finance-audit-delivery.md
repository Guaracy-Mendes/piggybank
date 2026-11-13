# Durable Finance audit delivery

Date: 2026-11-12. WP-24 remains **in progress**. This increment prevents damaged audit records from starving later events and makes delivery depend on native Controls evidence. It does not enable periodic financial posting or customer enrollment.

## Problem and ownership

The previous relay always selected the lowest fifty undelivered UUIDs and returned on the first invalid record. A retained malformed record could prevent every later event from reaching Controls. The new real PostgreSQL regression reproduces that failure before the fix.

Finance owns immutable original pending records, delivery attempts, progress and receipt links. Controls owns the original append-only audit stream and hash chain. Core remains the sole financial authority; audit delivery never changes a balance, journal, reservation, fee, policy approval or financial receipt. Native basis audit events retain their basis UUID, original actor and capture time.

Forward [migration 0052](../../migrations/0052_finance_audit_delivery.sql) adds a singleton cursor, immutable delivery attempts, immutable native receipt links, guarded acknowledgment and verified legacy backfill. Applied migrations 0001–0051 remain unchanged. Source records and prior delivery timestamps are preserved.

## Relay algorithm

1. Read the persisted cursor. Select at most fifty undelivered events after it, in UUID order. An empty page resets the cursor to zero for a later pass. A retained legacy zero-UUID event is inspected once at wrapping, so it is neither skipped forever nor allowed to monopolize normal passes.
2. Before attempting each identity, commit that identity and scan time as progress. A crash resumes after the attempted identity; wrapping eventually retries earlier failures and late lower-ID inserts. Concurrent workers may overlap; original native identity makes overlap idempotent.
3. Parse the original record with the shared Controls validator. Require its exact typed/canonical representation to match the retained raw JSON. Invalid/noncanonical records remain unchanged and pending.
4. Independently inquire through `finance.confirm_audit_delivery`. If Controls already committed the exact original event, recover its receipt without appending again.
5. If no original event is visible, append the original ID, Finance stream and original typed payload through Controls. An exception, cancellation or timeout does not establish failure. Independently inquire again.
6. Confirm delivery only when the native event has the original ID, `finance` stream, exact original payload and a valid native digest computation. Atomically insert the immutable Finance receipt and acknowledge the pending row. Identity/payload/stream conflicts cannot become success.
7. Retain a safe immutable attempt with original source digest, observation code and native attempt time. Failed attempts remain observable while later events progress. The pass has a two-second work budget checked between events and a two-second per-event timeout; cursor reads, progress writes and attempt persistence use the existing database settings outside that per-event timeout. This is a bounded batch and asynchronous event budget, not a total-duration SLA. No database transaction spans the Controls append and Finance acknowledgment.

The cursor advances per attempted identity, rather than past an unattempted batch suffix. This preserves fairness when the work budget expires. Event IDs and occurrence times never change on retry. A timeout can race a commit; native receipt inquiry and original-ID replay resolve that uncertainty.

| Observation code | Meaning |
| --- | --- |
| `NATIVE_RECEIPT_CONFIRMED` | Native evidence was observed and acknowledged |
| `INVALID_RECORD` | Original JSON or audit fields fail the shared validator |
| `NON_CANONICAL_RECORD` | Parsing would change the original representation; it is not silently rewritten |
| `AUDIT_IDENTITY_CONFLICT` | Native identity, stream, payload or receipt integrity conflicts |
| `CONFIRMATION_UNAVAILABLE` | Confirmation was unavailable or timed out; original identity retained |
| `APPEND_UNCONFIRMED` | Append was attempted, but the original native receipt was not observed |

Native receipt evidence takes precedence over an earlier uncertain attempt. An attempt observation alone never proves delivery.

## Native receipt and privilege boundary

`finance.confirm_audit_delivery(uuid)` locks the original pending row, checks immutable native Controls evidence, stores original Controls chain digest/sequence and confirmation time, then marks delivery in one transaction. It accepts no caller-supplied payload, digest, sequence or success flag. Finance cannot directly insert/update/delete/truncate receipts. Inserting a source with a forged delivery timestamp or updating delivery without matching native receipt evidence is rejected by guarded triggers.

`finance.audit_delivery_receipt(uuid)` is a read-only, scope-bound native verifier. Every item inquiry checks original ID, stream, payload, chain digest/sequence and the event's digest computation against the saved receipt. Missing or damaged native links fail closed. Original record, latest attempt and verified receipt are read in one statement snapshot so an inquiry cannot combine an old pending state with a later confirmed attempt.

The native check verifies the referenced event and its saved link. Whole-stream verification remains the existing Controls `AuditLog::verify` capability; this increment does not claim an external audit anchor or independent backup certification.

The migration verifies legacy delivered rows against their original Controls events before backfilling `LEGACY_VERIFIED` receipts. It leaves original `delivered_at` values unchanged and records the new verification time separately. A legacy flag without matching native evidence aborts the migration transaction. Operators must investigate or restore authoritative evidence; no synthetic receipt or manual forced flag is created.

## Inquiry API and staff view

Both endpoints require workforce authentication and current Finance role/queue authority. There are no mutation endpoints.

| Method and path | Result |
| --- | --- |
| `GET /api/v1/staff/finance/audit-delivery` | Live pending/confirmed counts, observed UTC time, persisted cursor/scan time and up to 100 safe items, pending first |
| `GET /api/v1/staff/finance/audit-delivery/{id}` | Original safe metadata, latest attempt and verified native receipt, even outside the list window |

Counts and item reads are live observations and can see different commits. Counts and native sequence numbers are exact decimal strings. Item state is `PENDING`, `PENDING_RETRY` or `DELIVERED`; delivery requires the verified native receipt. Responses expose safe IDs, action codes and digests, with no raw actor identities, raw payloads, authentication material or signatures. A malformed record does not expose arbitrary text through an action label.

The **Audit delivery** workforce view displays original identity, retained pending records, safe attempts, original resource, native receipt digest/sequence and separate delivery/confirmation clocks. Saved URLs use `#/finance/audit/{auditId}`. It polls every four seconds, scopes async reads to the current selection and invalidates them on unmount. Failed inquiries clear previously displayed receipt/state data. A confirmed receipt remains authoritative even when an optional relay-attempt observation was not retained. There is no control to delete a failed event, rewrite its payload, force delivery or replay money.

See the [operator runbook](../runbooks/finance-audit-delivery.md), [checkpoint evidence](evidence/WP-24-audit-delivery.md), [C4 components](current-finance-components.puml), [relay sequence](current-finance-audit-delivery.puml) and [domain UML](current-finance-audit-domain.puml).
