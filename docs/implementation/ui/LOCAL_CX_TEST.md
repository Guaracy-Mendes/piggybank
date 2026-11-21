# Local end-to-end customer experience test

This preview runs the current source with the official HTML visual language and native synthetic consumer/investment capabilities. It is separate from the sealed schema-56 cloud release. TST, cards, bank links, asset prices, securities and corporate actions are simulated; use synthetic identity information only.

## Start or resume

From the repository root:

```sh
make dev-up
make identity-setup
python3 scripts/local_cx.py prepare
python3 scripts/local_cx.py start
```

`prepare` builds the Rust applications, applies forward migrations and seeds independently approved chart v3 without replacing v1/v2 or existing financial records. `start` owns only its preview processes and refuses occupied ports. It persists after the terminal exits. Customer portal: <http://127.0.0.1:4300>. Staff: <http://127.0.0.1:4301>. Use `127.0.0.1` consistently because issuer redirects and cookies are configured for that host.

Customer accounts: `alice` or `bob`, password `Synthetic-Local-123!`. Staff accounts: `reviewer`, `approver`, `security`, with the same local fixture password. These credentials apply only to the supplied synthetic local issuer configuration.

If an account is already active, reuse it. Otherwise sign in, choose **Open an account**, save a synthetic profile and `example.invalid` contact, attach the synthetic evidence text, accept the actual terms and submit. The local worker completes its synthetic checks. Registration, SMS/email delivery, physical biometrics and real KYC provider qualification are separate pending capabilities; the issuer-backed fixture accounts are ready for this CX test.

## Step-by-step journey

1. Open **Overview**. Check the greeting, total personal net worth, personal vaults, investment allocation and recent activity. Toggle net-worth visibility. Shared pools are excluded from personal net worth because the implementation does not invent ownership percentages.
2. Open **Cards & wallet**. Under **Link a simulated bank**, enter `Local Evaluation Bank`, label `Test funds`, last four digits `1234`. Connect it. In the connected bank card, choose your account and add `500.00` TST. Review the exact deposit, choose **Confirm identity to authorize**, sign in at the issuer again, then **Submit approved instruction**. Expect `POSTED` and a native batch ID. The simulator has a lifetime allowance of TST 1,000 per person across all links; opening another link does not reset it.
3. Open **Savings & vaults → New vault**. Name it `Rainy Day`, target `500.00`, optionally set a date, then create it. Add `50.00` from the funded bank account. Verify the balance, goal progress and vault-specific ledger. Lock it and check that withdrawal is refused; unlock before a valid withdrawal.
4. Create a virtual card under **Cards & wallet**. Check that freezing disables purchase submission and that unfreezing restores it. Revealing the synthetic token requires a fresh issuer approval; it does not return a real PAN/CVV.
5. Open **Savings & vaults** and configure round-ups for that card and vault, multiplier `1×`, weekly cap `10.00`. Review and authorize at the issuer. Return to **Cards & wallet**, enter merchant `Local Coffee`, category `Dining`, purchase `7.50`, and review/authorize. Expect the separate native `0.50` saving, subject to available funds and the cap; the round-up panel shows the trailing-30-day and lifetime totals. A failed saving never reverses a completed purchase.
6. Open **Spending & insights**. Check the full-month spending total, category allocation and top merchants. Under **Budgets**, set Dining to `20.00`, choose rollover and an alert threshold. Under **Activity**, search/filter and open the purchase. Change its category/note or attach a PNG/JPEG/PDF receipt up to 32 KiB. Financial journal facts remain immutable.
7. Open **Investments**, explicitly accept the synthetic brokerage terms, then open it. Deposit `200.00` from your bank. Select Apple (`AAPL`), choose **Buy**, enter `25.00` and review the exact fractional quantity, approved price bound and fee. Reauthenticate at the issuer and submit. Expect `FILLED` only with a native fill batch, actual units and execution amount. One minor unit of valuation rounding can differ from booked purchase cost; all calculations use exact minor units and microshares.
8. Open the same asset and configure a **Limit** order below its fixture price, for example `1.00` TST per share. A valid buy order stays `OPEN`, reserves only its approved cash and has no fill receipt. Cancel it under **Recent & open orders** and check buying power is restored once. A filled order cannot be cancelled.
9. Open **Automated wealth**. Create a `5.00` recurring SAVE or INVEST instruction with today’s UTC date, review its complete terms and approve it at the issuer. The worker checks current ownership and retained consent, posts at most one schedule/date effect and advances the next date. Use Pause/Resume/Cancel to inspect state. Choose **Edit instruction** to change the amount, cadence or next date; the change needs a new issuer approval, keeps the paused/active state and leaves the previous version under **Show previous versions & cancelled schedules** as `REPLACED`. Open **Execution receipts** for the five most recent native runs and check the monthly commitment and executed totals. Worker changes refresh while the tab is visible; the refresh button also reads the native state.
10. Add an asset to **Watchlist & income** and set a synthetic price alert. DRIP requires its own exact issuer authorization. Dividend cash is credited only for a declared action and actual record-time holdings; the next seeded corporate actions occur in the following UTC month. No past dividends or returns are invented.
11. Open **Preferences & API**. Save light/dark/system appearance and goals. Create a scoped read token through issuer approval; copy it once, then revoke it. A revoked token or a token used for another scope returns `401`. Tokens never authorize payment/trade commands.
12. Return to **Overview**. After the vault deposits above, the header shows a `1-Day Saver` streak link and the hero shows the change since the UTC month start; **Net worth by month** lists the last six month starts and today. Hide net worth and check the change is masked too. Open the streak link (or **Savings & vaults → View streaks & badges**): the heatmap marks today, **First deposit** is unlocked with today's date and locked badges show their progress. Choose **Create milestone card**; the preview shows only the badge and date. Use **Save image** (or **Share card** where the browser supports file sharing). The **Interest & disclosures** card on the Savings hub shows lifetime interest from actual periodic postings only.
13. Open **Statements & documents**. The current UTC month shows *Available after the month closes*; closed months show *Ready to create* or an existing version. Choose **Create statement**, then **View**: opening + money in − money out = closing, with the content SHA-256. **Download PDF** uses a new single-use link that expires in two minutes. Submit a data request (copy, correction or deletion) and check it shows *In review*. As staff `reviewer`, open **Records & retention**, enter the account identifier, place a legal hold, try **Request disposal** (refused: retention), and process the request; as `approver`, release the hold. The [records runbook](../../runbooks/statements-retention.md) gives every step. The bounded custom activity report (CSV, JSON or PDF) remains under **Preferences & API**.
14. Open **Notifications**, filter categories or unread entries and mark one/all as read. Under **Messages**, the statement you created produces one safe message (no amounts). Turn **Payment activity** off under **Message preferences**; security and statement notices stay on. As staff, **Operations monitor** shows a fixed snapshot, data-quality alerts, monitoring alerts and message delivery (see the [event replay runbook](../../runbooks/event-replay.md)). Check help/support and existing account/periodic disclosure links remain reachable.
15. Open **Profile & security**: **Sign out everywhere** revokes every session, including the native app, and sends a security notice. Staff use **Security** for recovery, access reviews and findings (see the [incident runbook](../../runbooks/security-incident.md)).
16. For the dedicated native app, follow the [mobile development runbook](../../runbooks/mobile-development.md) (Android emulator; iOS pending).
17. Repeat the main navigation with browser responsive mode at **390×844** or **375×812**. Check the five bottom tabs, forms, charts, horizontal table scrolling, keyboard focus and light/dark appearance. A phone on another machine cannot use the loopback URL directly; use this local browser or an explicitly configured secure remote preview.

## Recovery and expected boundaries

- Keep the approval tab on the same `127.0.0.1` origin. If its three-minute approval expires, obtain a new review; changing a request key cannot reuse a completed authorization.
- Native commands retain their original request key through retries. Repeating the same key/input returns its original receipt, and changed input under that key is refused.
- Open orders, holds, vault balances, tokens, notifications and schedules survive a preview restart. This preview never clears the persistent database.
- Prices are observed synthetic fixtures, not a live WebSocket market feed. Existing charts show only retained observations. Benchmarks, analyst research, real securities, physical device authentication and PRD latency/FPS targets are not certified by this local preview.
- The current local database is schema 80; see the [local deployment runbook](../../runbooks/local-deployment.md) for upgrades. Historical sealed schema-56 images require a separate compatible database; there is no automatic schema downgrade.
- Saved bills are reminders and cancellation requests. They do not claim merchant payment or cancellation confirmation. Incoming payment requests never move money without the existing reviewed payment flow.

```sh
python3 scripts/local_cx.py status
python3 scripts/local_cx.py stop
python3 scripts/local_cx.py start
```

Only the preview’s own processes are stopped. PostgreSQL, Keycloak, NATS, their data and unrelated applications remain intact. Logs: `.local/cx-runtime.log`. Stop the preview before automated browser suites because those suites own an isolated database and the same loopback ports.
