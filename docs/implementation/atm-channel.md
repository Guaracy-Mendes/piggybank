# WP-35 ATM channel supplement (local simulator)

WP-35 adds migration `0069_atm_channel.sql` (schema 69). It covers the ATM part of FR-CH-04 and the local ATM branches
of AT-22, as a **local synthetic R2 capability** against a deterministic network simulator: the acceptance test plays
the network. There is no real network, no cash logistics and no card issuing. Actual network certification and a
chosen network (DEC-04, DEC-09) are required before any enablement.

## Confirmed operations (immutable fixture)

| Operation | Status |
| --- | --- |
| `BALANCE_INQUIRY` | Enabled: returns the Core available balance, or declines a non-active account |
| `CASH_WITHDRAWAL` | Enabled: TST 500.00 per withdrawal, TST 1,000.00 per card per business day, in multiples of TST 10.00 |
| `DEPOSIT`, `TRANSFER`, `PIN_CHANGE`, `MINI_STATEMENT` | Declined `OPERATION_NOT_SUPPORTED`, with no effect |

## Channel authentication and keys

- **Messages.** Each network message is signed with HMAC-SHA256 over its exact body (`Piggybank-Atm-Signature`), and
  every response is signed back. A missing or wrong signature returns 401 and nothing is processed.
- **PIN.** The network must assert `pinVerified`; without it the result is `CHANNEL_AUTHENTICATION_FAILED`.
- **Cards.** A card is a synthetic token (`ATM-` plus 16 hex digits) that a customer links to an account they own.
  Delegates and assisted staff cannot create one. It works only while its holder still owns the account, and the
  customer can block it permanently.
- **Key.** The network key is a synthetic constant in the local profile (SF-005).

## Lifecycle and financial keys

| Message | Effect |
| --- | --- |
| `AUTHORIZATION` | An authorization row is written first (`PENDING`, keyed by the network message id). Core then reserves a hold whose operation is the authorization id, giving `APPROVED`. Otherwise the result is `DECLINED`: `LIMIT_EXCEEDED`, `ACCOUNT_RESTRICTED` or `INSUFFICIENT_FUNDS`. The authorization expires after 120 s. |
| `COMPLETION` | `core.atm_withdrawal` posts customer → ATM settlement account (`ATM_WITHDRAWAL`), consuming the hold. `UNIQUE(operation, kind)` means any number of completions post once. |
| `REVERSAL` before completion | Releases the hold (`REVERSED`). A later completion is declined with `AUTHORIZATION_NOT_OPEN`. |
| `REVERSAL` after completion | Never auto-posted. Recorded as a late reversal (`REVIEW_REQUIRED`) and surfaced by reconciliation. |
| Network timeout | The worker expires uncompleted authorizations and releases their holds (`EXPIRED`). |
| Duplicate message id | The stored, signed response is replayed byte for byte. The same id with different content returns 409. |

Common restrictions come from Core itself: the hold and the posting re-check account state, debit blocks and funds.
A customer freeze or an assisted protective freeze therefore declines ATM withdrawals too.

## Reconciliation

A signed settlement file (`/channels/atm/v1/settlement-files`) is compared with the bank's completed withdrawals for
the business day. Every item is classified `MATCHED`, `MISSING_AT_BANK`, `MISSING_AT_NETWORK`, `AMOUNT_MISMATCH` or
`LATE_REVERSAL_REVIEW`, and nothing is netted. Files are immutable and idempotent by file id.

## Interfaces

| Audience | Endpoints and pages |
| --- | --- |
| Network | `POST /channels/atm/v1/messages`, `POST /channels/atm/v1/settlement-files` |
| Customer | `GET`/`POST /api/v1/atm/cards`, `POST /api/v1/atm/cards/{id}/block`. Customer portal: **ATM access** in **Profile & security**. |
| Staff | `GET /api/v1/staff/atm` (payment inquiry authority). Staff portal: **ATM channel**. |

The model is [current-atm-lifecycle.puml](current-atm-lifecycle.puml).

## Not in scope

- Real ATM networks, card issuing, PIN management, EMV and cryptograms, cash logistics and terminal management.
- Partial dispense and deposits.
- Network certification, settlement with a real network, and fees.
