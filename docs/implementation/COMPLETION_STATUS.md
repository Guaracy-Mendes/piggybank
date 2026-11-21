# Implementation completion status

Updated 2026-11-21.

## Official UI and wealth extension

A separate user-authorized [official UI and wealth extension](ui/ARCHITECTURE.md) is in progress. Schema 76 (2026-11-19) adds expense splits with paid-request linking, favourites, merchant saving rules and a labelled safe-to-spend figure; schema 77 adds vault photo goals; QR payment codes complete UI-22 sharing (no migration); schema 78 counts merchant saving rules toward saving streaks and badges; self-service registration at the issuer completes UI-02; schema 79 adds email verification by one-time code with simulated delivery (UI-03). On 2026-11-21 a shared session fix stopped a window-focus re-check from remounting both portals and discarding page state; it was the root cause of the recurring milestones test failure. Schema 80 adds savings interest under user-approved synthetic product rules (base 3.00%, a TST 50,000 cap, stacking saving-streak and new-customer boosts up to 4.50%, a one-off challenge and referral bonuses), completing UI-09 and UI-14. 26 of 32 screens are now locally implemented ([evidence](ui/EVIDENCE.md)). Its [32-screen coverage](ui/SCREEN_COVERAGE.md) and [local CX test](ui/LOCAL_CX_TEST.md) are tracked separately from the 41-package roadmap below. The new source does not change the historical essential WP-24 completion or the sealed schema-56 archive.

| Screen status | Count | Screens |
| --- | ---: | --- |
| Locally implemented | 26 | All 32 except those below |
| Locally partial | 5 | UI-04 KYC (live identity verification, deferred by the user); UI-24 (real bank-transfer provider); UI-27, UI-28 and UI-31 (live market, research and benchmark feeds) |
| Pending a device | 1 | UI-05 biometrics and passcode (a physical device) |
| Total | 32 | PRD screens |

26/32 = **81% by unweighted screen count**. UI-09 and UI-14 use user-approved synthetic rules that still need a jurisdictional (Brazil) review before any real use. Every remaining screen needs an external input; none can be finished locally.

## Roadmap packages

**WP-41 is complete** within the user-chosen local capacity template. KPI-01..08 are immutable snapshots with explicit denominators and exclusions, labelled local synthetic. One measured change took burst intent timeouts from 21,970 to 0 on the same host. ADR 0002 keeps the modular monolith ([evidence](evidence/WP-41.md), [specification](outcomes-capacity.md)). Production KPIs, cost and capacity remain pending (DEC-07, DEC-12). AT-18 burst stays FAIL: the host delivered only 86% of the 3x schedule. **WP-32 is blocked** behind WP-31 and launch decisions. **WP-40 is complete** locally: reproducible liquidity forecasts from signed liquidity evidence and warehouse history at a recorded cut, with confirmed funds only from fresh signed evidence. Policies are versioned and maker/checker. A breach restricts new external submissions and opens an owned Finance case. Settlement prefunding is proposed only by a confirmed breach and needs a maker and a different approver; Core books it once within approved per-action and daily limits ([evidence](evidence/WP-40.md), [specification](liquidity-forecasting.md)). Real treasury feeds and accuracy against real outcomes remain pending (DEC-03/06/07/12). **WP-39 is complete** with user-directed scope: a template with two simulated providers for the same rail. Route policies are versioned and changed by maker/checker. Provider health is measured from recorded simulator outcomes. Fallback happens only on signed, single-use proof that the original instruction was not executed, with the same operation, amount and hold, so a payment cannot run on two providers. Unknown outcomes are inquired, never re-routed. Shadow evaluation and compatible rollback precede activation ([evidence](evidence/WP-39.md), [specification](payment-routing.md)). Benefit against a real baseline remains pending (AT-28, DEC-03/04/07/11). **WP-38 is complete** with user-directed scope: one local journey-partner simulator role as the template for selected partners. Consent-checked audiences use minimized signals and pseudonymous tokens. Advisory insights carry provenance and a customer-facing why. The signed partner channel accepts advice only. Withdrawal hides and withholds without deleting evidence, and outcome reports state denominators, watermarks and late data ([evidence](evidence/WP-38.md), [specification](audiences-journeys.md)). Real partners remain pending (DEC-05/06/09). **WP-37 is complete** with user-approved scope: one local warehouse adapter as the template for later analytical adapters. It has versioned datasets with backward-compatible schemas, ingestion deduplicated by economic identity, reproducible snapshots with lineage, purpose checks, completeness flags and Core control totals, and no Core write path ([evidence](evidence/WP-37.md), [specification](governed-warehouse.md)). Analytical products remain pending selection (DEC-05/06/08). **WP-36 is complete** with user-approved scope: one synthetic ACH-style batch credit rail as the template for later rails. It has a cutoff, calendar, T+1 settlement and returns, with common approval and entry-keyed postings ([evidence](evidence/WP-36.md), [supplement](batch-rail.md)). Other rails remain pending selection. **WP-35 is complete** against a local ATM network simulator role: a confirmed-operations supplement, signed channel messages, holds and single postings keyed by authorization, duplicate replay, reversal and timeout handling, and item-by-item settlement reconciliation ([evidence](evidence/WP-35.md), [supplement](atm-channel.md)). **WP-34 is complete** as a local synthetic R2 capability: assisted branch, private-banker and call-center sessions the customer authorizes in a bank channel, with only confirmed operations (read and protective freeze; no payments until DEC-09), common restrictions and recorded staff/customer/channel authority ([evidence](evidence/WP-34.md), [specification](assisted-channels.md)). **WP-33 is complete** as a local synthetic R2 capability: registered partner clients with maker/checker approval, scoped and revocable customer consent re-checked at every decision, a versioned contract, quotas, signed callbacks, partner-drafted payments that only the customer can approve and that survive revocation (AT-21), and bounded delegated account authority ([evidence](evidence/WP-33.md), [specification](partner-consents.md)). Production enablement still requires WP-32. **WP-30 is complete** as a local single-host qualification. The branch-level acceptance manifest has 30 scenarios and 54 branches: PASS 41, FAIL 1, PENDING 7, NOT_APPLICABLE_R1 5. The sustained NFR-02 to NFR-05 workload passed for 600 s, and resilience drills D1–D5 passed. **The 5-minute 3x burst fails for payment intents on this host** (CF-04, recorded as AT-18 FAIL), after two serializations found during qualification were fixed. See the [qualification report](local-qualification.md) and [evidence](evidence/WP-30.md). **WP-29 is complete** within its local synthetic scope: fail-closed configuration, locked expand-only migrations, a gated local CI-equivalent pipeline, metrics, paging with escalation, and a backup/restore drill ([evidence](evidence/WP-29.md), [specification](operations-tooling.md)). **WP-31 is blocked** on real provider, market and production decisions (DEC-01, DEC-03 to DEC-07, DEC-10 to DEC-12); simulator results cannot satisfy it. WP-32 depends on it. **WP-28 is complete** within its local synthetic scope: issuer-backed recovery with independent approval, sealed session tokens, privacy restriction/export, access recertification and an independently dispositioned findings register ([evidence](evidence/WP-28.md), [specification](security-lifecycle.md), [threat model](../security/threat-model.md)). **WP-27 is complete** on an Android emulator with iOS explicitly pending: native app with system-browser PKCE, Keystore-held rotating tokens, native action-bound step-up and persisted-operation recovery ([evidence](evidence/WP-27.md), [specification](native-mobile.md), [runbook](../runbooks/mobile-development.md)). **WP-26 is complete** within its local synthetic scope: transaction monitoring with independent disposition, owned data-quality alerts with balance-safe repair, fixed-snapshot operations reports, traces and safe notifications ([evidence](evidence/WP-26.md), [specification](operations-monitoring.md), [runbook](../runbooks/event-replay.md)). **WP-25 is complete** within its local synthetic scope: readiness-gated statement versions, single-use retrieval, fixture retention, legal holds, evidenced disposal and data-subject requests. See the [WP-25 evidence](evidence/WP-25.md), [specification](statements-retention.md), [runbook](../runbooks/statements-retention.md) and [authoritative board](WORK_PACKAGES.md). WP-24 remains complete for its [user-approved essential scope](wp24-essential-scope.md) ([evidence](evidence/WP-24-complete.md)). No package is ready: WP-31 and WP-32 need external decisions.

| Status | Count | Packages |
| --- | ---: | --- |
| Complete within documented local scope | 39 | WP-01–WP-30, WP-33–WP-41 |
| In progress | 0 | None |
| Blocked | 2 | WP-31 (external decisions and providers), WP-32 (depends on WP-31 and launch decisions) |
| Planned | 0 | None |
| Total | 41 | Full roadmap |

39/41 = **95.1% by unweighted package count**. Package sizes differ; this is not an effort or production-readiness percentage. R0 has 9/9 packages complete; R1 has 21/23 complete; WP-31 and WP-32 are blocked. R2 has 6/6 complete locally (WP-33 to WP-38). R3 has 3/3 complete locally (WP-39 to WP-41; WP-41 as a local template). The full R1 release remains incomplete.

## Implemented essentials

- Platform contracts, identity/session/CSRF/current authority, immutable audit/evidence, durable operations and events.
- Native balanced ledger, exact money, holds/restrictions, versioned products, parties/accounts, onboarding, account-opening terms, servicing, customer/staff web.
- Internal transfers; simulated external reservation/submission/settlement; incoming suspense/allocation; linked returns, cancellation and native receipt recovery.
- Unique balanced GL exports/authenticated recovery; immutable reports/revisions; explicit liquidity freshness/completeness; outgoing/incoming/grouped reconciliation with owned differences.
- Issuer-backed account recovery with risk signals and independent approval; AES-256-GCM sealing of server-held session tokens; privacy restriction and personal export; access recertification and an independently dispositioned findings register.
- Native Android app (iOS pending) sharing the API contract, with system-browser PKCE, OS-protected rotating credentials, native action-bound step-up and persisted-operation recovery.
- Retrospective transaction monitoring with one alert per economic episode and independent disposition; owned data-quality alerts with read-model-only repair; fixed-snapshot operations reports, durable-identifier traces and deduplicated safe notifications.
- Monthly statement versions from native journals with readiness gates and supersession; two-minute single-use retrieval with access evidence; fixture retention, independently released legal holds, evidenced disposal and per-record data-subject outcomes.
- Independently approved periodic financial terms, explicit attributable owner consent and pending withdrawal, separate native activation approval, sequential immutable account-period finality, exact fees/accruals, automatic durable processing, one native effect and linked approved corrections.

## Deferred enhancements and remaining roadmap

The [scope decision](wp24-essential-scope.md) explicitly defers non-UTC/DST, more complex product rules, scale beyond 1,000 source facts, global close/restatement, resuming stopped enrollment, real legal certification and production/provider/cloud qualification. Historical preview acknowledgments remain distinct from financial consent. No requirement is silently removed from the full PRD.

| Packages | Remaining work |
| --- | --- |
| WP-31–WP-32 | Real providers and production boundaries (blocked), controlled R1 readiness |
| CF-04 | 3x burst capacity for payment intents beyond this single host (DEC-07, WP-41) |
| WP-36–WP-41 | Complete as local templates; further rails, analytical products, journey partners, a real routing baseline, and production KPIs/cost need selection (DEC-03 to DEC-09, DEC-12) |
| UI extension | 6 of 32 screens need live KYC, a device, a bank-transfer provider or market feeds; the savings interest rules need a Brazil-specific legal review |

## Current verification

Latest local run: 2026-11-21, schema 80. Gated pipeline `.local/ci/20261121T145513Z` (14:55–15:23 UTC) passed 37 of 37 steps ([UI evidence](ui/EVIDENCE.md)). The load stage is not part of the gated run; it last ran in `.local/ci/20261118T004715Z` ([WP-30 evidence](evidence/WP-30.md)). The cloud archive row still refers to the sealed schema-56 release.

| Check | Executed result |
| --- | --- |
| Integration suite | 176 passed, 0 failed |
| Library tests | 21 passed, plus 2 API binary tests |
| Acceptance manifest | Valid: 30 scenarios, 79 branches (PASS 62, FAIL 1, PENDING 16) |
| R2/R3 acceptance (WP-33 to WP-40) | Partner consents, assisted channels, ATM channel, batch rail, governed warehouse, audiences and journeys, payment routing, liquidity forecasting and outcome KPIs all passed |
| Load (local single host, 2026-11-18) | Sustained 600 s at 200 reads/s and 50 commands/s passed: read p95 18.2 ms, command p95 46.9 ms. Core 958 transfers/s, p95 20.1 ms. The 5-minute 3x burst **failed** for payment intents (about 69/s of 75). Invariants held. |
| Resilience | D1 worker crash, D2 API crash, D3 broker outage, D4 issuer outage and D5 provider kill all passed, with invariants held |
| Native mobile | Native API contract and Android emulator journey (Pixel 9 Pro XL AVD) passed; iOS pending |
| Security | Real-issuer identity and security acceptance passed |
| Browser regression | 38 flows passed: 21 consumer (including QR codes, registration, email verification and savings interest), 14 payment/Finance, and one each for customer, workforce and issuer |
| HTTP acceptance | All 251 implemented ingress paths passed |
| Source / contracts / portals | Formatting, locked Clippy with warnings denied, implemented OpenAPI, generated client, strict web types and both portal builds passed |
| Cloud / infrastructure | Last verified for the sealed schema-56 archive on 2026-11-16; not rerun since |
| Models / documents | All 83 C4/UML sources passed the PlantUML syntax check; 128 Markdown files with 1,057 local links, requirement and UI-source checks passed (32 screens mapped) |
| Immutable history | 80 migrations locked by SHA-384 (`make migrations-check`) |

Source state: work through schema 79 was committed as `d79f033` on branch `local-roadmap-and-ui-extension` (`main` is unchanged at `2b61cee`); the session revalidation fix and schema 80 follow on the same branch.

## Deployment boundary

`cloud-20261115-01` is the accepted schema-56 synthetic Linux/amd64 archive. Its SHA256 is `b2bdb5d49570fe2c5ea1d6012d11a635f5acb6cead70f28aa7b4f9917ac66bd2`. The [GCP](../deployment/gcp/RUNBOOK.md) and [AWS](../deployment/aws/RUNBOOK.md) runbooks and deployment files are ready for evaluation; the actual sealed images passed local acceptance. No live cloud infrastructure was applied and no production launch was qualified. Historical releases and migrations remain sealed.
