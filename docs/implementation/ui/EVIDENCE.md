# Official UI and wealth — local CX acceptance

Recorded 2026-11-16; updated for the schema-60 recurring-revision and schema-61 savings-progress increments. The current source is ready for the core local synthetic CX journey. The complete 32-screen production PRD remains in progress; [screen coverage](SCREEN_COVERAGE.md) records the exact implemented behavior and remaining requirements. This extension is separate from essential WP-24 acceptance and the sealed schema-56 cloud archive.

## Executed acceptance

| Check | Actual result |
| --- | --- |
| Workspace integration regression | 149 cases passed across nine test binaries, including the original 97 Finance cases and 22 Core cases (six recurring-revision and three savings-progress scenarios) |
| Native consumer regression | All 19 consumer cases passed within the Core binary, including the approval-expiry/lock-wait scenario |
| Library regression | All 19 library cases passed |
| Native PDF structure/escaping test | One binary test passed; complete multipage facts, escaped literals and xref offset validated |
| New consumer browser suite | Four actual-issuer journeys passed, including savings milestones (streak, monthly change, heatmap, badges, card) in light/dark/mobile, recurring edit/approval/version history and execution receipts, plus including vault funding, budgets, brokerage funding, fractional fill, Finance GL booking, card freeze/purchase/round-up, finite linked funding, scoped token revocation, native PDF, dark appearance and notifications |
| Banking/Finance browser regression | All 14 distinct existing journeys accepted across the full run and targeted reruns of two corrected receipt contrast cases |
| Customer, staff and issuer browser regressions | Three distinct journeys passed; customer lifecycle also retains exact balances beyond JavaScript Number precision and responsive navigation |
| Total accepted browser journeys | 21 distinct flows (consumer 4, payments/Finance 14, customer 1, staff 1, issuer 1); actual local issuer, native PostgreSQL and persistent simulators |
| Accessibility | WCAG-tagged Axe checks in the browser flows, including desktop/mobile consumer and dark preferences; not blanket certification of every pending screen/device |
| Contract/client synchronization | 138 actual implemented HTTP route patterns checked; generated TypeScript client synchronized |
| Ingress | 138 patterns passed safe error, size-limit, shared two-instance quota and forwarded-address-spoof checks |
| Static/production builds | Workspace Rust formatting and strict Clippy passed; shared/customer/staff TypeScript checks and both portal production builds passed |
| PlantUML | All 62 diagrams rendered and XML-validated, including the updated consumer state, financial-domain and personal-domain views; all 62 syntax checks passed |
| Provenance | Six official source files unchanged; all 32 PRD screens explicitly mapped; four font/license assets match retained SHA-256 provenance |
| Documentation | Local links and the canonical 22-diagram PRD gallery synchronized and checked |

The new consumer suite uses a separate isolated database from the older banking/Finance suite. The latter deliberately retains damaged source and identity fixtures, so new-customer acceptance cannot share its final state. No financial assertion was weakened to hide those fault-injection effects. Legacy greeting/mobile-navigation assertions were updated for the user-authorized new shell. Contrast fixes preserve the official palette while making selected rows and receipts readable.

## Recurring revision increment (schema 60)

Migration 0060 adds editable recurring SAVE/INVEST instructions without mutating approved terms. An edit requires a fresh issuer approval bound to the digest of the unchanged reviewed basis. The original row, consent and completed runs remain immutable; the replacement keeps the family identity, pause state and monthly anchor day. A separate immutable `(family, due date)` index allows at most one effect per date across all versions, a run completed after review invalidates the revision, and the next date must follow the last completed family run. The investment snapshot adds per-version and family execution receipts derived from native journal entries and a cadence-based monthly commitment summary. The consumer snapshot adds trailing-30-day round-up totals and per-member shared vault contributions labelled by ordinal.

The new browser journey initially failed on two test-only issues, both corrected without weakening any assertion: Playwright's exact label match includes a wrapped `<select>`'s selected value, so role-based locators are now used; and code chips inherited muted text on the soft background (about 4.3:1). `.ui-code` now uses the ink token in both appearances, and the Axe check passes.

## Savings progress increment (schema 61)

Migration 0061 adds a derived progress read to the workspace snapshot; it stores nothing and posts nothing. Contribution facts are driven from the reader's own deposit commands, SAVE runs and round-up facts (indexed), so other people's vault activity is never scanned or attributed. Pure UTC date-island functions compute current/longest daily and monthly streaks and badge unlock dates; their database test covers month and year boundaries, the next-day grace, broken runs and ignored future dates. The integration test verifies attribution, a goal badge retained after withdrawal, month-start net worth from retained facts, that journal plus position-event replay equals the live dashboard total after a fill, an incomplete result for a holding with no retained price, lifetime interest net of a linked correction, isolation from another person, and the autopilot badge after one recurring run.

The new browser journey covers the Home streak link, monthly change pill and month bars, the Savings interest summary, the milestones heatmap and badges, and a locally generated card in light, dark and 390×844 layouts. Axe found two contrast defects, both fixed in shared styles: muted captions on the pale unlocked-badge background (unlocked badges now use the card surface with an accent edge), and the hard-coded `.text-link` color in dark appearance, which also affected the existing Savings "New vault" link. One early run lost the card preview to a Vite dependency-optimization reload; six consecutive repeat attempts and the following full runs rendered it within 10 ms.

## Split, favourites, saving rules and safe-to-spend increment (schema 76)

Migration 0076, added 2026-11-19, wraps the consumer command, review and snapshot functions in the same way as
schemas 59 to 61. Core remains the only money authority.

| Screen | What was added |
| --- | --- |
| UI-20 | `SPLIT_EXPENSE` splits an outgoing transaction on the customer's own account between one and nine other customers. Shares are equal in exact minor units. When the requester is included, the requester keeps the remainder; otherwise the first participants pay one more minor unit. Each participant gets a request; a request is never a transfer. `REQUEST_MARK_PAID` lets the payer link a settled internal payment of the exact share from their own account to the requester's account. A payment can settle only one request. |
| UI-22 | `FAVORITE_SET` / `FAVORITE_REMOVE` store up to 20 favourite receiving references; only known references can be saved. They are offered on the request and split forms. |
| UI-13 | `SAVE_RULE_SET` requires an issuer-approved review, like round-ups. Each matching card purchase (merchant or category, case-insensitive) moves the fixed amount from the card's account to the vault, once per purchase and rule (deterministic operation id), within the weekly cap. A failed rule saving notifies the customer and never reverses the purchase. Rules can only be disabled, not edited. Rule savings (`MERCHANT_SAVE`) are excluded from spending analytics, as round-ups are. |
| UI-17 | The snapshot adds `safeToSpend`: available balance minus approved automations and bill reminders due by month end. It is labelled as a local rule, not advice. |

| Check | Result |
| --- | --- |
| Core database test `splits_paid_requests_favourites_saving_rules_and_safe_to_spend_are_native_and_replay_safe` | Passed: exact shares and remainder, replay, one split per transaction, invalid participants, receivers cannot split incoming money, paid-request linking (wrong amount, wrong linker and a foreign payment refused), favourites, rule approval, weekly cap, replay without double saving, non-matching merchants, disable-only rules, safe-to-spend arithmetic |
| Core suite | 27/27 passed |
| Browser journeys | Split with a favourite and safe-to-spend (two customers, axe). The wallet journey now also approves a merchant saving rule through the real issuer, and its saving is excluded from spending (750 spent, 100 saved). |

**Gated runs.** The first gated run (`.local/ci/20261119T220753Z`) failed at the migration lint after 6 steps, before
any test ran: 0076 renamed functions, which the expand-only rule forbids. 0076 was rewritten with `CREATE OR REPLACE`
functions that delegate to the same underlying functions as the schema-59 to schema-61 wrappers. The first rewrite
dropped the schema-60 recurring-schedule review path; four Core recurring tests caught it and it was restored. The
second gated run (`.local/ci/20261119T221237Z`, 2026-11-19 22:12–22:39 UTC) passed all 37 steps:
- integration: 172 passed, 0 failed;
- consumer browser: 17 passed;
- migrated to schema 76, then locked.

**Still pending.**
- QR sharing (UI-22): done in the QR payment codes increment below. Camera scanning still needs a device.
- Saving rules did not count toward savings streaks. Done in schema 78 (below).

## Vault photo goals increment (schema 77)

Migration 0077 (2026-11-19) completes UI-10. `VAULT_PHOTO` lets the vault owner attach one PNG or JPEG of at most
32 KiB to a vault, replace it or remove it. Members of a shared vault see it. The photo is presentation metadata only;
it never changes a balance or the journal. 0077 extends the schema-76 functions with `CREATE OR REPLACE` (no renames).

| Check | Result |
| --- | --- |
| Core database test `vault_photo_goals_are_bounded_owner_only_metadata` | Passed: format, size, base64 and field checks; owner-only changes; no visibility for non-members; removal; no journal effect. Core suite 28/28. |
| Browser journey "vault photo goal: ..." | Passed, with axe |
| Gated pipeline (`.local/ci/20261119T224323Z`, 22:43–23:09 UTC) | All 37 steps passed on the first run: integration 173/0, consumer browser 18, migrated to schema 77 and locked |

## QR payment codes increment (no migration)

On 2026-11-19 the user allowed adding `qrcode-generator` 1.4.4 (MIT, no dependencies), pinned exactly in
`apps/customer-web/package.json` and the lockfile. This completes UI-22 sharing without a schema change.

- `apps/customer-web/src/lib/paymentCode.ts` defines the code text `pbtst:pay?ref=<reference>[&amount=<minor>][&memo=<text>]`.
  The parser accepts only that form or a bare `PB`/`SIM` reference. It rejects unknown or repeated keys, bad amounts
  and long memos rather than guessing.
- `apps/customer-web/src/components/QrCode.tsx` draws the library's module grid as one SVG path (error correction M,
  4-module quiet zone). No generated markup is injected.
- "Share to receive" shows the receiving reference as a QR code. An open outgoing request can be shared as a QR code
  carrying the requester's reference, amount and memo.
- The recipient form accepts the decoded code text ("Use payment code"). It only fills in the reference and shows the
  requested amount; adding the recipient, quoting and issuer approval are unchanged. A code cannot move money.
- The grey captions in "Share to receive" failed axe colour contrast once a reference was on screen; they now use the
  body ink.

| Check | Result |
| --- | --- |
| Browser journey "QR payment codes: ..." | Passed, with axe. The test re-encodes each payload independently with the library and requires the rendered SVG path and viewBox to match it module for module. It also checks that invalid code text is refused, valid text fills the reference, and the request code carries `amount=1250&memo=Tickets`. |

**Gated runs.**
- `.local/ci/20261120T010137Z`: failed at the consumer browser step (18 passed, 1 failed). The new QR test clicked
  "Share this request" by text, and an open request left by the earlier split journey made the match ambiguous. The
  click is now scoped to the request's own card.
- `.local/ci/20261120T012129Z`: consumer browser passed 19/19 and integration passed, but the payments browser step
  failed (13 passed, 1 failed). The QR caption added a second `<code>` element inside the receiving-reference panel,
  so the existing test selector `.receiving-reference code` matched two elements. The caption is now a `span`.
- After that fix, the failed payments test and the QR test both passed when run on their own.
- `.local/ci/20261120T162431Z` (2026-11-20 16:24–16:51 UTC, after a host reboot): all 37 steps passed. Consumer
  browser passed 19/19 and payments browser 14/14.

**Still pending.** In-app camera scanning needs a device camera (device qualification).

## Saving-rule contributions increment (schema 78)

Migration 0078 (2026-11-20) closes the gap left by schema 76: approved merchant saving rules now count as savings
contributions. `core.consumer_contribution_facts` (schema 61) feeds the saving streaks, the saving heatmap, the
FIRST_SAVE, STREAK and SAVED_1K badges, and the lifetime contributed total. It is replaced with `CREATE OR REPLACE`
under the same signature, adding source `SAVE_RULE` from `core.consumer_save_rule_facts`. Each fact is one posted
`MERCHANT_SAVE` batch, so a saving is counted exactly once. The AUTO_SAVER badge stays tied to scheduled saves and
ROUNDUPS_100 to round-ups, so existing badge meanings are unchanged. A person index on the facts table supports the
read. Rule savings remain excluded from spending analytics.

| Check | Result |
| --- | --- |
| Core test `splits_paid_requests_favourites_saving_rules_and_safe_to_spend_are_native_and_replay_safe` | Extended: after two rule savings (TST 2.00 + TST 1.00), contributions rise by 2, lifetime contributed rises by 300 minor units, and the current saving streak is at least one day. Core suite 28/28. |
| Negative check | With 0078 removed, the same test fails (contributions rose by 0, expected 2). The migration was restored. |
| Gated pipeline (`.local/ci/20261120T165325Z`, 16:53–17:19 UTC) | All 37 steps passed on the first run: integration 173/0, consumer browser 19, migrated to schema 78 and locked |

## Self-service registration increment (UI-02, no migration)

On 2026-11-20 the user approved changing the shared local Keycloak customers realm. Credentials are entered only at the
issuer; customer-web never sees a password.

- **Entry point.** "Create account" on the welcome page goes to the new customer-only route `/auth/customer/register`
  (`beginRegistration` in the contract). It sends the same PKCE, state and nonce login request as sign-in, plus OIDC
  `prompt=create`. `/auth/workforce/register` and `/auth/service/register` return 404.
- **Realm** (`infra/identity/customers-realm.json`, `scripts/identity.py`):
  - password policy `length(8) and maxLength(128) and upperCase(1) and specialChars(1) and notUsername and notEmail`;
  - login theme `piggybank`;
  - the registration flow's terms step is required;
  - new users get the `customer` role as a default role.
  Registration is on unless `BANK_CUSTOMER_REGISTRATION=disabled`, which the cloud bootstrap sets. The terms step is
  in the registration flow only, so existing users are never re-prompted.
- **Theme** (`infra/identity/themes/piggybank`). It inherits the stock Keycloak v2 pages. On the registration page it
  adds a short intro and a live strength meter: a `meter` element, an `aria-live` status and a checklist of 8+
  characters, an uppercase letter and a symbol. Synthetic-demo terms text is added. The meter only guides; the realm
  policy decides. The theme is mounted read-only in the local compose file, the cloud compose file and the deployment
  runtime test.
- **After registration.** The new identity returns signed in with the `CUSTOMER` trust context. It has no person link,
  so it can only read the catalogue and start an application. Accounts follow approved onboarding as before.
- **Not collected.** There is no referral code: nothing would use it until referral rewards are approved (UI-14).

| Check | Result |
| --- | --- |
| Browser journey "self-service registration: ..." | Passed, with axe on the signed-in app. It checks Weak, Good and Strong meter states, that the issuer refuses a password without an uppercase letter even when the meter is bypassed, terms acceptance, the return signed in and `/auth/customer/me` = `CUSTOMER`. It then deletes the synthetic user through the admin API; no `registration-*` user remains. |
| Issuer integration (`tests/integration/identity.py`) | Passed. The register redirect carries `prompt=create`, S256 PKCE, the `customer-web` client and the login-state cookie; the workforce and service register routes return 404. |
| Deployment checks (`tests/deployment/check.py`) | 6 tests passed |

**Gated runs.**
- `.local/ci/20261120T180154Z`: failed at the consumer browser step (19 passed, 1 failed). The savings-milestones
  journey's share-card image did not appear within 5 s. The registration journey, the issuer integration test and
  integration (173/0) passed.
- This is the second time that test has failed this way; the first was `.local/ci/20261118T163917Z`, recorded under
  WP-33. It passed in the full consumer suite locally (20/20) and in the schema-78 run.
- No root cause was proven. The card is drawn and PNG-encoded locally on a canvas, which is the only asynchronous step.
  Two changes were made:
  - the button now shows "Creating card…" and is disabled while the card is generated;
  - the test allows 15 s for the image.
- The milestones captions now say that merchant saving rules count (schema 78).
- `.local/ci/20261120T182947Z` (2026-11-20 18:29–18:57 UTC): all 37 steps passed. Integration 173/0, consumer
  browser 20/20 (including registration and milestones), payments browser 14/14. Afterwards the customers realm held
  only `alice` and `bob`.

**Incident, recovered.** The first realm change also set `registrationEmailAsUsername`. Keycloak then rewrote existing
usernames to their emails (`alice` became `alice@example.invalid`), and the issuer integration test's `alice` sign-in
failed with `user_not_found`. The admin event log showed exactly one user deletion, the test's own registration, so no
user was lost. The setting was turned off. Alice's username was restored by allowing username edits for one call and
then setting `editUsernameAllowed` back to its original `false`. Bob already showed `bob`. User IDs, and so issuer
subjects and person links, never changed. The form now asks for a username and an email separately.

**Still pending.**
- Email ownership verification (OTP provider, UI-03).
- Production abuse controls (CAPTCHA, rate limits).
- Registration in the cloud template, which stays off.

## Email verification increment (UI-03, schema 79)

This is a local template with simulated delivery, following the project's simulated-provider pattern. No real email or
SMS is sent.

- **Data and functions** (migration 0079):
  - `identity.contact_codes` stores salted SHA-256 digests only, with 10-minute expiry, 5 attempts, a 45-second resend
    gap, supersession and single use.
  - `simulator.contact_outbox` is the synthetic mailbox. It is written only by `simulator.deliver_contact_code`, inside
    the same transaction that issues the code.
  - `bank_identity` can call `contact_code_status`, `contact_code_issue` and `contact_code_check`, and nothing else.
- **API** (`crates/identity/src/contact.rs`, customer only; workforce and service get 404):
  - `GET /auth/customer/contact-verification` returns the masked email the issuer holds, whether it is verified, and
    the pending code's timers.
  - `POST` with CSRF sends a code: 409 if already verified, 429 inside the resend gap or past 5 per hour.
  - `POST .../confirm` checks a code. A match is accepted only if the issuer email is unchanged; it then sets
    `emailVerified` through `bank-identity-admin` and writes `identity.audit` `contact.email-verified`.
- **UI** (Profile & security, "Verify your email"):
  - the masked destination;
  - six boxes that advance automatically, accept a pasted or autofilled code (`one-time-code`) and support backspace;
  - a "Resend code in Ns" countdown, attempts left, and an expiry time.
  New registrations now land here (UI-02 → UI-03 → application).
- **Reading a local code for a demo:**
  `SELECT body FROM simulator.contact_outbox WHERE destination='<email>' ORDER BY created_at DESC LIMIT 1;`

| Check | Result |
| --- | --- |
| Identity database test `contact_codes_are_hashed_rate_limited_attempt_limited_single_use_and_expiring` (new `crates/identity/tests/database.rs`) | Passed: no direct table access for the identity role; digest is not the code or its unsalted hash; exactly one outbox message; resend gap; 4 misses then lock, after which even the right code is refused; supersession; single use; 10-minute expiry; 5 per hour; subject isolation |
| Issuer integration | Passed: alice reads `verified: true` with a masked destination; send without CSRF returns 403 and with CSRF returns 409 (already verified); workforce returns 404; unauthenticated returns 401 |
| Browser journey "self-service registration and email verification: ..." | Passed, with axe. After registering, the user lands on verification and sends a code; the resend button is disabled with a countdown; a wrong code shows 4 attempts left; pasting the outbox code into the first box verifies; the issuer user then has `emailVerified: true`. The synthetic user is deleted afterwards. |

**Gated runs.**
- `.local/ci/20261120T190641Z`: failed at the ingress step after every test suite had passed (integration 174/0,
  consumer browser 20/20, payments browser 14/14). The ingress probe called `POST .../confirm` without a JSON body and
  got 415, which the contract did not list.
- The contract now lists 415, 422 and 429 for confirm (as for other JSON routes) and 429 for the status read. The
  ingress check then passed locally.
- `.local/ci/20261120T193034Z` (2026-11-20 19:30–19:56 UTC): all 37 steps passed. Integration 174/0, consumer
  browser 20/20, payments browser 14/14. Migrated to schema 79 and locked.

**Still pending.** A real email/SMS provider, an SMS channel with phone numbers, and a second sign-in factor (this is
contact verification only).

## Native financial coverage

The final consumer regression verifies native vault funding/withdrawal/lock and same-key replay; foreign ownership refusal; exact step-up and one fill through concurrent retries; limit reservations/cancellation; fill/cancel concurrency; capped round-ups and card freeze; authorization reuse refusal; effective notification preference; unanimous shared withdrawals and anti-bypass guard; recurring mandate/date uniqueness under concurrent ticks; immutable record-time dividends and DRIP once; scoped/hash-only/revocable API credentials; complete owner-only statement facts; empty onboarding reads without financial mutation authority; and proof expiry during a real native cash-lock wait without an order, hold, position or cash change.

The real-issuer investment flow additionally waits for the original fill’s Finance export to become `BOOKED` under the approved chart. Submitted/open orders are not presented as financial fills.

## Persistent preview and upgrade

The local database was upgraded forward from schema 56 to 59, then 60 and 61 with `local_cx.py prepare`. Every one of the 61 applied SHA-384 checksums matches its current source file. Migrations 0057–0061 are now applied and immutable; future changes require a new migration. Chart v3 was published through the existing distinct maker/checker fixture identities; chart v1/v2 were retained.

The preview was restarted with `python3 scripts/local_cx.py start` after the schema-61 upgrade; API readiness and both portals responded and the unauthenticated investment read returned 401. The earlier schema-59 smoke used the actual customer issuer, authenticated reads of both consumer and investment workspaces, desktop rendering and 390×844 mobile navigation with no document overflow or page errors. It submitted no customer financial command to the persistent database. The preview remains running for the user. It owns only its API, worker, simulator and two portal processes; dependency containers and unrelated applications remain untouched.

Eight inactive disposable databases were removed only after confirming this task’s `urn:consumer-fixture` provenance and no active sessions. Successful browser fixtures cleaned their own apps/databases. Historical sealed archives were not rebuilt or replaced.

## Reproduce

Stop the persistent preview before browser suites:

```sh
python3 scripts/local_cx.py stop
BANK_TEST_ADMIN_URL=postgres://dev_admin:synthetic-admin-only@127.0.0.1:15432/piggybank scripts/cargo.sh test --workspace --locked --test database -- --test-threads=2
scripts/cargo.sh test --workspace --locked --lib
scripts/cargo.sh test --locked -p bank-api --bin bank-api
make test-consumer
make test-payments
make test-customer
make test-staff
make test-browser
make test-ingress
make app-check
make web-build
make ui-sources
make docs
python3 scripts/local_cx.py start
```

Shared PostgreSQL/browser suites run sequentially. Raw execution logs and local screenshots are retained under `.local/ui-*`, `.local/official-investments-*`, `.local/customer-*`, `.local/staff-*` and `.local/cx-*`. Use the [CX walkthrough](LOCAL_CX_TEST.md) for the human review.

No check here certifies live brokerage, real bank/card/KYC providers, SMS/email OTP, physical biometrics, market research/benchmarks, insurance/membership, a native mobile release, cloud deployment, or the PRD’s quantitative latency/FPS targets. Those requirements and the remaining richer UI/automation interactions stay explicit in the screen record.
