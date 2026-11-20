# Official consumer UI and native synthetic wealth

The current implementation extends the completed essential WP-24 banking baseline. Visual authority is the six retained user files in `mock/`; the HTML’s emerald/slate-teal palette overrides the supplied PRD’s conflicting coral/indigo tokens. Source hashes and the exact remaining work for every PRD screen are in [backlog.json](backlog.json) and [screen coverage](SCREEN_COVERAGE.md).

## Runtime and ownership

- `packages/web-core/src/official.css` supplies the shared design language, locally bundled licensed fonts, responsive cards/sidebar, mobile tabs and appearance variables. No external font/CDN request is needed by the application.
- `apps/customer-web/src/consumer/` supplies the native workspace context, personal finance surfaces and investment suite. Monetary arithmetic uses `BigInt`; normalized chart coordinates are presentation values. A visible tab refreshes server state every two seconds; this is not a qualified live market feed or push transport.
- `apps/bank-api/src/consumer.rs` preserves the existing customer BFF session, OIDC trust and CSRF controls. UUID request keys bind exact command inputs. Unlinked customers receive an empty onboarding read; financial mutations still require an authoritative canonical person.
- `crates/core/src/consumer.rs` exposes narrow native capabilities. Forward migrations 0057–0061 own the new records. Module-role direct writes to financial state are revoked; native security-definer functions use fixed search paths and validate current canonical actor, resource ownership, amounts and original consent.
- The existing worker recovers due native instructions. Finance discovers the original Core journals and exports them under independently approved chart v3. Chart v1/v2 and the sealed schema-56 release remain unchanged.

## Financial invariants

Bank accounts, personal/shared vaults and brokerage cash use Core’s authoritative booked/held balances and balanced journals. Vaults and brokerage cash are separate child ledgers, not free-money UI balances. Opening a brokerage creates zero cash and zero positions. Funding comes from an active owned bank account. The linked-bank simulator has one finite TST 1,000 lifetime evaluation allowance per person across all links, and each approved deposit posts through the explicit simulator clearing account.

Securities use integer microshares (one share = 1,000,000 units). Native quotes retain full terms and their digest. Buy costs round upward to a minor unit; sell proceeds and displayed valuations round downward. Cash reservations and share reservations are serialized by canonical person. An `OPEN` order has no fill receipt. Fills atomically retain the native balanced cash batch, position change, immutable position event, fill and outbox; cancellation releases only that open order’s original reservation. Average cost and realized gains are synthetic accounting values, not qualified real-market tax lots.

One exact issuer approval executes one command. Its native proof binds issuer, subject, client, action, material digest and expiry. Same-key retries return the original result; changed input under the same key is refused, and a completed authorization cannot be reused under another key. Token secrets are returned once and omitted from retained command receipts. Card reveal returns a clearly synthetic token, not a real PAN/CVV.

Shared withdrawals reserve Core funds, require every currently active member’s approval, and recheck the requester’s payout ownership. Disabling mutual approval cannot bypass an existing shared membership. Vault locks block payout. Each shared vault reports lifetime attributable posted contributions per active member (deposits, recurring saves and round-ups), labelling other members by ordinal rather than identity. Round-ups apply a configured multiplier and weekly cap to synthetic card purchases; their separate failure cannot erase an already posted purchase.

Recurring SAVE/INVEST instructions retain the approved mandate and current actor. The native worker checks current ownership and mandate fields, records one immutable `(schedule, due date)` effect and advances the cadence. Pause/resume/cancel are durable; resuming skips paused historical dates. Failures retain safe review codes and bounded retry times. Editing an active or paused instruction requires a fresh issuer-approved review of the unchanged reviewed basis (digest-bound); the original row, consent and runs stay immutable, the new version inherits the family identity, pause state and monthly anchor day, and one `(family, due date)` index prevents a second effect across versions. A run completed after the review invalidates that revision, and the next date must follow the last completed family run. Monthly cadence keeps the original anchor day, clamped to short months. The investment snapshot reports per-version and family execution receipts from native journal entries and a cadence-based monthly commitment projection. Declared synthetic dividends use immutable record-time position events and unique person/action evidence. DRIP requires a separate reviewed approval and reinvests only the actual credited dividend. Zero entitlements are recorded so empty accounts cannot starve later holders.

## Records and observations

Full-month spending analytics aggregate the complete native source period independently of the bounded recent-activity list. Vault/broker internal funding is excluded from spending. Category metadata never edits a financial journal. Budgets support one prior month’s unused rollover and native threshold notifications. Bills retain reminders and explicit cancellation requests; merchant confirmation is not invented.

Savings progress, monthly net worth and lifetime interest (migration 0061) are derived on each workspace read and store nothing. Contribution facts are positive vault-side entries attributable to the reader: their own `VAULT_DEPOSIT` commands, their recurring SAVE runs and their round-ups; withdrawals never remove an earned badge. Streaks are UTC date and calendar-month islands; a daily streak stays current through the next UTC day. The eight badges are a fixed catalogue whose unlock date is the first business date a threshold was met. Month-start net worth replays journal entries and immutable position events before the instant under the same personal definitions as the dashboard and current ownership; each held symbol uses the latest retained price observation at or before the instant, else the reader's own latest fill price, and is otherwise reported as missing so the monthly change is withheld. Lifetime interest sums `PERIODIC_ACCRUAL` postings to currently owned accounts net of linked corrections. The shareable milestone card is drawn in the browser without balances or identity.

Statements retain a complete bounded native source snapshot, opening/closing minor-unit balances, period, account, timestamp and SHA-256 digest. The capability rechecks ownership and digest before read/export. Ranges over 366 days or 1,000 facts are refused rather than silently truncated. JSON, formula-safe quoted CSV and a self-contained multipage PDF expose the same retained facts. This custom activity report is separate from the WP-25 [monthly statement versions](../statements-retention.md) shown under **Statements & documents**, which add readiness gates, supersession, single-use retrieval, retention and legal holds.

Personal API tokens are hash-only, scoped to one of three read capabilities, expire after 30 days and are revocable. Their dedicated endpoint never accepts them for payments, trades or other mutations. Preferences, notifications, links, bills, requests, card controls and vault goals persist server-side.

## HTTP surface

| Method | Path | Native capability |
| --- | --- | --- |
| GET | `/api/v1/consumer` | Current owned workspace and full-period analytical report |
| POST | `/api/v1/consumer/commands` | Exact action/input with original UUID idempotency key |
| POST | `/api/v1/consumer/authorizations` | Retain full action terms and create issuer-bound review |
| GET | `/api/v1/consumer/authorizations/{id}` | Owned exact review, proof readiness and retained non-secret receipt |
| GET | `/api/v1/consumer/statements/{id}` | Owned complete retained statement and integrity digest |
| GET | `/api/v1/consumer/statements/{id}/pdf` | Complete escaped multipage native-fact PDF |
| GET | `/api/v1/consumer/integrations/{scope}` | One matching unexpired read-only token scope |
| GET | `/api/v1/investments` | Owned brokerage, exact holdings/orders, observations and synthetic catalogue |
| POST | `/api/v1/investments/quotes` | Exact native fractional market/limit quote and authorization |

The OpenAPI surface and generated TypeScript client are synchronized. Existing account, product, payment, consent, Finance, staff and support capabilities remain in their original modules.

## Current diagrams

| View | PlantUML source | Render |
| --- | --- | --- |
| C4 L2 containers | [source](../current-consumer-containers.puml) | [SVG](../current-consumer-containers.svg) |
| C4 L3 components | [source](../current-consumer-components.puml) | [SVG](../current-consumer-components.svg) |
| UML personal finance domain | [source](../current-consumer-personal-domain.puml) | [SVG](../current-consumer-personal-domain.svg) |
| UML instruction state machines | [source](../current-consumer-states.puml) | [SVG](../current-consumer-states.svg) |
| UML financial domain | [source](../current-consumer-domain.puml) | [SVG](../current-consumer-domain.svg) |
| UML trade approval and execution sequence | [source](../current-consumer-trade.puml) | [SVG](../current-consumer-trade.svg) |

## Local acceptance and next scope

Use the [CX runbook](LOCAL_CX_TEST.md). Native financial regressions run against disposable PostgreSQL databases; browser acceptance uses the actual local issuer and isolated fixture. The consumer browser suite has its own database because older fault-injection suites deliberately retain damaged source/identity fixtures.

The current source is a reviewable synthetic implementation, not completion of every production requirement in all 32 screens. Registration/OTP delivery, native biometrics, real KYC/bank/card/broker adapters, market/research/benchmark feeds, some richer visual/automation interactions, physical-device qualification and quantitative latency/FPS targets remain explicitly mapped in the screen record. No template illustration establishes insurance, regulatory membership, actual securities custody or live market execution.
