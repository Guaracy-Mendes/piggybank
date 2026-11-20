# Batch rail acceptance (WP-36)

`batch.py` acts as the synthetic batch network against the real local issuer, API and worker on a fresh fixture
database: `make test-batch`.

| Result key | What is proven |
| --- | --- |
| `common_approval_before_hold` | A bad routing checksum and an over-limit entry are refused. Confirmation without step-up is refused. After native step-up, the entry is queued with one hold. |
| `cancel_before_cutoff_releases_hold` | A queued entry is cancelled and its hold released. After submission, cancellation is refused. |
| `rejection_releases_hold_and_replay` | A bad signature returns 401. An acknowledgement rejection (R03) releases the hold. A duplicate replays identical signed bytes, and reusing an id returns 409. |
| `settlement_posts_once` | Two settlement messages produce one `BATCH_CREDIT` per entry. |
| `return_inside_window_posts_once_linked` | Two returns produce one `BATCH_RETURN`, linked to its credit, and the money is back on the account. |
| `late_return_flagged_not_posted` | After back-dating one settled entry's settlement date (a database clock fixture), a return is flagged for review with no posting. An unknown reason code is declined. |

Ledger invariants are checked at the end, and the batch clearing account equals settled minus returned.
