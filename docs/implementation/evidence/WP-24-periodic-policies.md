# WP-24 checkpoint: periodic policy configuration and exact previews

Date: 2026-11-08. Package status: **in progress**. This increment adds configuration/approval and calculation previews, not periodic financial posting.

## Implemented behavior

- Separate immutable Core policy versions bind an exact product, disjoint finite effective range, complete policy-scoped UTC calendar and material fee/accrual calculation. The registry is a narrow configuration capability; FinanceRecords remains a read-only financial port.
- Whole-policy canonical digests, separate PERIODIC_POLICY purpose, independent canonical people, current maker/checker grants/queues and expiry are enforced. The native guarded function recomputes the approved digest and serializes version publication.
- Finance persists original intent/key/reason/expiry before Controls registration. A wrapping 50-request cursor repairs lost registration acknowledgments past revoked commands; original native publication receipts recover a lost response.
- Monthly whole-calendar fixed fees and daily ACT/365-fixed accruals use canonical signed-i64 Money strings and i128 numerators. Holiday/weekend following roll, leap day, exact cutoff equality and zero/half-unit previews are explicit.
- Every policy is SYNTHETIC/PREVIEW_ONLY. Operator-supplied balance previews never post journals or enroll customer accounts. Existing payment fees and prior V1 digests are preserved.
- Five route patterns / seven operations add current-role-protected staff proposal/detail/approval/publication/calculation. Total implemented inventory: 102 patterns. The portal exposes complete material review, exact-string amounts, original publication receipts and saved URLs.
- A complete operator runbook, two JSON input examples, C4 components and separate sequence/domain UML document the implementation.

Migrations **0048_periodic_preview_policies.sql** and **0049_periodic_publication_clock.sql** are applied to persistent local PostgreSQL and are immutable. The second is a forward change that rechecks live approval/grant expiry after waiting for the native rule lock and records the actual publication clock; it preserves migration 0048. Earlier applied migrations remain unchanged. See the [specification](../periodic-accounting-policies.md), [runbook](../../runbooks/periodic-accounting-policies.md), [C4](../current-finance-components.puml), [sequence](../current-periodic-policies.puml) and [domain UML](../current-periodic-policy-domain.puml).

## Verification

Current Rust formatting/Clippy/OpenAPI/generated-client/TypeScript checks pass. The final expanded suite passed all 84 integration-target cases across nine targets, including 51 Finance cases (83 database-backed cases and one pure exact-total case). It includes expiry after a native publication-lock wait. All 15 library cases pass, including five exact-math/calendar/wire tests. All eight payment/Finance browser flows, one customer lifecycle and one workforce regression passed. The final focused periodic browser flow also passed after the explicit UTC clock display change. The actual sealed cloud archive also passed packaged-runtime acceptance.

The seven periodic database cases cover durable concurrent replay and original native acknowledgment, canonical identity aliases, immutable request/key material, unpublished-preview denial and unrelated roles, current role/queue revocation and approval purpose, sequential disjoint version races and date boundaries, a cursor beyond fifty invalid makers, raw native document/hash binding, runtime privilege boundaries, expiry and grant/approval expiration during an actual lock wait. Native journals/balances are unchanged by policy and preview workflows.

| Executed check | Actual result |
| --- | --- |
| `make app-check` | Formatting, locked Clippy with warnings denied, real OpenAPI validation, generated client drift and TypeScript passed |
| `make test-unit` | 15 library cases passed; exact arithmetic, leap/holiday/weekend/cutoff/wire/invalid-calendar tests include rejection of leap-second cutoffs |
| `make test-integration` | 84 cases across nine targets, including seven periodic database scenarios, passed |
| `make test-payments` | All eight real-issuer payment/Finance browser flows passed, including deliberate stale-response delivery |
| `make test-customer` / `make test-staff` | Customer lifecycle and independent workforce regression both passed |
| `make test-ingress` | 102 path patterns passed contract/safe errors/body/quota/address-spoof checks |
| Final periodic browser rerun | Passed exact UTC due/evaluation display, both calculation kinds, independent approval/publication, native non-mutation, roles, saved URL, mobile/accessibility |
| `make web-check` / `make web-build` | All web TypeScript checks and both compiled portals passed |
| Publisher / packing checks | Six transport helper and five packing/file checks passed |
| `make cloud-check` | All six configuration/archive tests passed |
| Diagrams / documentation | 42 sources rendered, XML-validated and syntax checked; all requirement and local-reference checks passed |

Tests sharing PostgreSQL run sequentially. The functional browser quota remains 5,000/minute; provider timeouts/bounds and current-identity controls are unchanged. The final mobile image `.local/finance-periodic-mobile.png` was visually inspected; desktop capture is `.local/finance-periodic-desktop.png`. Actual logs use `.local/periodic-*`; initial failure/before-fix logs are retained separately from final passing runs.

## Customer status race found during regression

A combined browser run exposed an authentic older DRAFT response replacing a newer accepted/settled payment status and disabling polling. The existing Core receipt was correct. A forced out-of-order-response test reproduced the failure before the fix. The customer detail now accepts only the current payment identity and a nondecreasing instruction version; equal-version independent read fields can still refresh. The forced-response test passes after the fix, retaining the original operation/journal and exact payer/recipient balances. The final combined eight-flow suite passed with the forced-response regression.

## Deployment refresh

The fresh archive `cloud-20261108-02` contains schema 49 and both current portals. It is 82,020,180 bytes, SHA256 `4dd19c7aae8dd0803d52005894c32537499f80b61a96e8030dbf67fbcc827ba0`, with 21 deployment manifest members and nested image SHA256 `02913df98a5e89c5b6651d9b12613f65ceca62d4be95b6bec501335fab69243e`. Actual archive verification/acceptance passed fresh credentials/identity/bootstrap, health/portals, both clearing namespaces, two closed groups, periodic channel boundaries, explicit UTC labels in the served archived workforce bundle and repeated immutable seed. Disposable resources were removed; developer services were retained. Full authenticated policy workflow was validated by host browsers, separately from packaged smoke. [Cloud validation](../../deployment/VALIDATION.md) retains exact image IDs and historical release seals.

No live GCP/AWS account or billable infrastructure was used. Both complete cloud runbooks and input examples point to this fresh identifier; existing archived releases remain unchanged.

## Remaining WP-24 work

Customer enrollment and exact material terms, authoritative historical Core basis, product/rule/period financial uniqueness, guarded native fee/accrual posting, approved counterpart/GL mappings, scheduled job/cutoff recovery and independently approved linked financial corrections remain unfinished. Non-UTC/DST production calendar qualification remains open. Local synthetic validation does not establish GCP/AWS deployment, provider certification, financial policy approval or production readiness.
