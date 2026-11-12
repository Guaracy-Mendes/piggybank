# WP-24 checkpoint: native periodic ledger basis observations

Date: 2026-11-11 (America/Porto_Velho). WP-24 remains **in progress**. Native observations and calculation previews are implemented; customer enrollment, authoritative economic finalization and periodic financial execution remain separate work.

## Implemented behavior

- Forward migration 0051 creates immutable Core basis evidence and guarded actor-key capture, plus a persisted Finance audit cursor. It is applied locally; migrations 0001–0051 are immutable.
- Exact published policy/account-product binding and frozen finite UTC calendar determine period and due date. No operator balance or evaluation clock is accepted.
- Account metadata/current position and committed native account journal prefix share one native statement MVCC view. Business-date opening/change/closing use exact i128 strings; current booked/held/available remain separate i64 strings.
- The entire prefix, including opening history, is limited to 1,000 facts. The 1,001st aborts without a partial artifact. Source items preserve original journal/operation/date/time/fingerprint/account delta and stable ordinal pages.
- Original actor/client/key replay retains native clock, facts, totals and digests. Fresh keys observe later commits without changing prior artifacts. Ended periods remain observations rather than global book close or authoritative economic finalization.
- Derived native-source previews retain the captured clock and explicit provisional status. Negative/out-of-i64 daily basis or account registration after period start suppresses calculation. No value is clamped or replaced by current booked balance.
- Every read verifies raw/typed/source/policy hashes and scope/totals, then independently checks saved native journal facts. Rehashed false stored amounts cannot become financial evidence.
- Current maker/canonical identity/REVIEW authority is rechecked after the native key lock. Audit uses the original basis UUID/person/time; fifty-UUID wrapping recovery repairs missing acknowledgment past damaged records without monetary work.
- The existing policy workbench now includes capture/retry/fresh actions, saved URLs bound to exact policy, source paging, historical versus current positions, exact-string money and UTC clocks. Checkers are read-only. Three new route patterns/four operations bring the implemented inventory to 109.

See [specification](../periodic-native-basis.md), [runbook](../../runbooks/periodic-native-basis.md), [C4](../current-finance-components.puml), [sequence](../current-periodic-native-basis.puml) and [domain UML](../current-periodic-basis-domain.puml).

## Verification in progress

Nine focused PostgreSQL cases and seven Core calculation unit cases passed before broad validation. Focused cases cover exact large amounts/holds/holiday due/replay; ended history versus current position; exact scope/role/queue/page guards; rehashed source tampering; 1,000/1,001 source bounds; concurrent actual financial commits; lost audit acknowledgment beyond fifty damaged artifacts; authority expiry during a real key-lock wait; and closed whole-month fee calculation without posting. Account registration dates are fixture metadata only; financial facts are generated through native Core commands and are not backdated or rewritten.

Full regression, browser workflow and fresh archive acceptance results will be recorded after execution. Actual working logs use `.local/basis-*`.

## Remaining scope

Captures never enroll accounts, establish customer consent or an amount owed, post fees/interest, consume/release holds or mark an economic period financially complete. WP-24 still requires material customer terms/enrollment, authoritative economic uniqueness/finalization, durable posting jobs, accepted native fee/accrual/counter-account commands and corresponding periodic correction eligibility. Funding-only corrections remain separate. No live GCP/AWS apply or production/provider qualification is claimed.
