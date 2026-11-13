# Finance audit delivery runbook

Use this procedure to distinguish retained pending events from confirmed native audit delivery. Audit delivery is separate from financial settlement and Core posting. Read the [implementation semantics](../implementation/finance-audit-delivery.md).

## Prepare

1. Complete [local development](local-development.md), or the [GCP](../deployment/gcp/RUNBOOK.md)/[AWS](../deployment/aws/RUNBOOK.md) deployment runbook and workstation tunnels. Use matching schema-52 binaries and migrations.
2. Verify API/worker readiness and a fresh workforce login. After restarting PostgreSQL, discovery can be available before issuer database connections recover; verify the actual login flow.
3. Sign in as `reviewer` or `approver` with current Finance inquiry role and REVIEW/APPROVE queue membership. Local password is `Synthetic-Local-123!`; a VM uses its per-install password retrieved privately through the cloud runbook. Security administration alone grants no Finance inquiry.
4. Keep the worker running. It owns bounded automatic delivery and original-identity recovery. No audit-recovery action requires a new financial command.

Expected: **Audit delivery** appears in the workforce navigation. Customer and unrelated workforce identities cannot read its API.

## Review a pending event

1. Open **Audit delivery**. Record counts, UTC observation time and last relay scan. These are live observations, not a sealed financial report.
2. Select an original event. The list contains up to 100 records, pending first in UUID order; a saved `#/finance/audit/<event UUID>` opens an event outside that window.
3. Record original audit UUID, resource UUID when available, original record digest and latest attempt/time. Raw actor/payload content is intentionally excluded from this response.
4. For `INVALID_RECORD` or `NON_CANONICAL_RECORD`, preserve the original and investigate its producing workflow or authoritative restored data. Do not edit its body, manufacture a replacement timestamp, delete it or set delivery flags manually.
5. For `AUDIT_IDENTITY_CONFLICT`, compare the original ID/stream/payload through an authorized investigation of native Controls evidence. The application cannot accept a different payload under the same ID.
6. For unavailable/unconfirmed outcomes, restore dependencies and let the worker continue. Repeated inquiry/replay retains the original event ID and occurrence time.

Expected: failed records remain pending while later valid events can reach Controls. Pages of fifty and the persisted wrapping cursor eventually revisit earlier failures and late lower-ID inserts. A cursor position alone does not establish an event's outcome.

## Verify native delivery

1. Open the original saved URL and select **Refresh audit delivery**.
2. Require state `DELIVERED` and the **Native audit receipt** panel. Inspect original native event ID, Controls chain digest/sequence and native confirmation UTC time.
3. Distinguish **Delivery acknowledged at** from **Receipt confirmed at**. A `LEGACY_VERIFIED` receipt preserves the old delivery time and records forward-migration verification separately.
4. Reload the URL or inspect it with the independent Finance checker. Expected: the same original record digest, event ID and native receipt, without any editable success flag.
5. For a lost append/acknowledgment, keep the original ID. If Controls already accepted it, native inquiry recovers the same event without appending a second one. Earlier uncertain attempt observations remain history.
6. Check financial completion independently through the original Core receipt and existing payment/correction runbooks. Audit delivery neither posts nor reverses money.

Expected: native evidence matches original ID, Finance stream and exact original payload. A damaged receipt link returns an integrity conflict and clears the prior UI proof; it does not silently become pending or delivered. A native receipt can be valid without an optional relay-attempt record, so inspect the receipt rather than inferring failure from missing attempt history.

## Direct inquiry

Using an existing workforce session or valid workforce bearer, without printing credentials:

```http
GET /api/v1/staff/finance/audit-delivery
GET /api/v1/staff/finance/audit-delivery/<original audit UUID>
```

Expected: authorized safe JSON, exact decimal-string counts/sequence numbers and nullable receipt/attempt fields. There is no POST/PATCH/DELETE delivery API. Keep session/bearer values and raw native audit payloads out of tickets and screenshots.

## Upgrade and recovery

1. Follow the cloud/local forward-upgrade and matched backup procedure before migration 0052. Do not edit earlier migrations.
2. The migration backfills a receipt only for an already-delivered Finance event with original matching Controls evidence and a valid native event digest. Expected: preserved old delivery timestamp and `LEGACY_VERIFIED` origin.
3. If backfill refuses missing/conflicting evidence, retain the failure and original database state. The migration rolls back. Investigate authoritative records or use a complete matching restore; do not invent a Controls event or force a flag to pass the gate.
4. After a worker restart, inspect persisted scan time and original IDs. No in-memory cursor or replacement identity is needed. The worker resumes and wraps automatically.
5. Use existing stopped full backups, including PostgreSQL and original deployment credentials/evidence. Delivery receipts do not substitute for a verified backup or external audit anchor.

Completion checklist: original UUID/digest retained; current Finance inquiry authority verified; malformed records retained; later valid delivery observed; native receipt verified; original replay/restart preserved identity; no raw payload leaked and no financial action used to repair audit delivery.
