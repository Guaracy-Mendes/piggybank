# Governed warehouse acceptance (WP-37)

`analytics.py` drives the staff analytics API against the real local issuer, API and worker on a fresh fixture database:
`make test-analytics`. The warehouse is the single local adapter (user decision 2026-11-18); no analytical product is
selected (DEC-05/06/08).

| Result key | What is proven |
| --- | --- |
| `missing_or_stale_sources_flagged` | A snapshot taken before the new posting is ingested is `INCOMPLETE` and does not reconcile with Core. |
| `backfill_replay_dedups_economically` | Backfills of both datasets are recorded. Repeating the ledger backfill inserts nothing and counts the earlier entries as duplicates. |
| `aggregate_traces_to_source_schema_transform_purpose` | The next snapshot is `COMPLETE`, reconciles with Core and supersedes the first. A funding row traces to its `transaction_views` source row, schema version 1, the ingestion run, the transform digest and the purpose. The snapshot reproduces from its stored cuts. |
| `purpose_denial_recorded` | A `MARKETING` snapshot returns 403 and appears in the recorded denials with `PURPOSE_NOT_PERMITTED_FOR_DATASET`. |
| `schema_upgrade_backward_compatible` | A version 2 that removes `currency` returns 409. A version 2 that adds optional `productVersionId` is accepted. New facts record version 2, earlier facts keep version 1, and a transform v2 snapshot lists both. |
| `analytics_cannot_change_core` | Core journal and balance totals change only by the posting made outside analytics. The Data role has no usage on the `core` schema. |

The database tests in `crates/data/tests/cases/analytics.rs` cover corrections, economic conflicts, master-reference
conflicts, privacy-restriction exclusion and the privilege audit in more depth.
