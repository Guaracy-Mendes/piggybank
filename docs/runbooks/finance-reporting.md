# Finance reporting and liquidity — synthetic local profile

Use workforce **Finance & GL** after `make migrate identity-setup dev-seed`. Snapshot creation requires the current Finance adjuster role and Finance REVIEW relationship. Independent Finance approvers can inquire. Snapshot creation records observations and has no Core money command.

## Save and reconstruct a report

1. Choose inclusive **Report from date** and exclusive **Report to date**, both UTC dates. Keep the period within 366 days and 1,000 Core batches. Narrow a rejected oversized period; do not replace missing rows with estimated totals.
2. Choose **Save Finance snapshot**. Retain the snapshot URL, artifact digest, Core/Finance as-of times, exact controls and source count. An uncertain response must retry the same request UUID and period. The browser retains that nonsecret key until success.
3. Inspect **GL completeness**, overall completeness, reason codes and **Frozen source lineage**. Only fully matched original receipts contribute to confirmed GL totals. A partial result explains missing, pending or conflicting evidence. Newly processing work has a visible 30 second synthetic timing window.
4. Follow the owned snapshot cases. A pending case link is recovered by the worker in bounded pages; refresh to see the link. Cases retain the original observation and are not erased when a later snapshot is complete.
5. Reopen the saved URL or page its lineage. Its Core source membership, controls, captured evidence and digests remain unchanged after later bookings or feed updates. Create another explicit snapshot for a later observation. Use the [controlled revision workflow](finance-revisions.md) to approve and publish a replacement link; this preserves both artifacts and their digests.

## Interpret liquidity

**Available liquidity** is shown only with fresh, authenticated independent evidence and a recent Core commitment observation. It subtracts all Core outgoing principal from provider available cash and floors at zero. This conservative local rule may reserve the same commitment again; it does not authorize a payment or release.

Core reserved/submitted principal, provider reserved cash, externally settled cash and expected incoming/outgoing are separate values. Expected funds are never added to available cash. Missing, invalid or stale evidence leaves current cash unavailable. A saved report may retain FRESH evidence at capture while its derived freshness or the live feed is now stale; the saved artifact describes its own as-of times.

The simulator publishes no cash amount by default. To exercise a feed, an authenticated local fixture supplies `LiquidityEvidence` at `POST http://127.0.0.1:4182/admin/liquidity-positions` with `X-Liquidity-Key`. Use the exact `SYNTHETIC_LIQUIDITY` provider, `SYNTHETIC_TST_CLEARING` scope, TST/scale-2 currency, unique evidence ID/version, as-of/valid-until interval of at most 60 seconds and canonical aggregate strings. `settled` must equal `available + reserved`. A stale fixture uses a past valid interval, rather than extending an old observation's expiry. Never infer this independent cash observation from a Core ledger balance.

## Expected results and completion checks

| Check | Expected result |
| --- | --- |
| Concurrent posting during Core capture | Journal membership and aggregate positions come from one consistent Core read transaction |
| Lost response or duplicate save request | Original artifact ID, digest, membership and captured status returned |
| Same key with another period | `409`; original artifact unchanged |
| More than 1,000 sources | `422`; no truncated artifact published |
| Changed GL value/date/mapping or missing receipt | Partial controls with explicit reason and owned observation; no claimed matching total |
| Missing/stale/invalid/latest bad provider version | Current available cash unavailable; no fallback to ledger or older feed values |
| Later bookings or new provider evidence | Earlier artifact unchanged; new explicit snapshot captures later membership/evidence |
| Revoked Finance role/queue | Inquiry and idempotency replay denied |

- Core debit and credit controls balance exactly; account controls reconstruct those totals.
- Matched GL controls include only authenticated original-scope receipts.
- Core and Finance source times, captured provider time/expiry and evidence references are retained.
- Paging returns the frozen membership without duplicate or newly appended sources.
- Snapshot issues have owned cases or a pending assignment that the worker can recover.
- Core journals, fees and held/customer balances were not changed by reporting or feed publication.

`make test-integration` covers real database concurrency, immutable membership/replay, exact controls, classification, current authority, missing/expired/invalid evidence and oversize rejection. `make test-payments` includes the real issuer/workforce report, liquidity, replay, accessibility and mobile workflow. These checks cover the synthetic observation slice, not statutory reports, a real cash provider or production qualification.
