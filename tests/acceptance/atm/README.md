# ATM channel acceptance (WP-35)

`atm.py` acts as the ATM network against the real local API and worker on a fresh fixture database. It signs every
message with the synthetic network key and verifies the signed responses. Run it with `make test-atm`; it takes about
two minutes, including a real 120-second authorization timeout.

| Result key | What is proven |
| --- | --- |
| `unsupported_operations_declined_without_effect` | A wrong signature returns 401. Without a PIN assertion the message is declined. Deposit, transfer, PIN change and mini statement are declined, and no hold changes. |
| `duplicate_completion_posts_once` | A duplicate authorization replays identical response bytes with one hold. Reusing a message id returns 409. Two completions give one `ATM_WITHDRAWAL` batch. |
| `reversal_and_late_reversal` | A reversal before completion releases the hold, and a later completion is refused. A reversal after completion is flagged for review with no posting. |
| `limits_enforced` | The per-withdrawal and daily limits apply. |
| `timeout_expiry_releases_hold` | The worker expires an uncompleted authorization and releases its hold. |
| `settlement_reconciliation` | A late reversal under review, an amount mismatch and a withdrawal missing at the bank are each listed. |
| `restriction_and_blocked_card_declined` | A frozen account is declined (`ACCOUNT_RESTRICTED`). A blocked card returns `CARD_NOT_ACTIVE`. |

Ledger invariants are checked at the end: journals balanced and booked balances equal to journal sums. The ATM
settlement account equals the completed withdrawals.
