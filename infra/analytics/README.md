# Analytical storage and processing adapters (WP-37)

Only one adapter exists: `LOCAL_WAREHOUSE` ([manifest](local-warehouse.json)). It is a template, built locally by user
decision on 2026-11-18. No analytical product is selected (DEC-05, DEC-06, DEC-08), and nothing here is a production
deployment.

A selected adapter must provide the same contract as the local one:

1. **Datasets and schemas.** Accept the dataset versions in `contracts/events/analytics/`, and enforce the same
   backward-compatibility rule. Every earlier field keeps its type and requiredness; new fields are optional.
2. **Facts.** Keep facts append-only and keyed by economic identity. Count re-delivered rows as duplicates, and quarantine
   different content for the same identity instead of overwriting it.
3. **Snapshots.** Record, for every snapshot:
   - the source fact cut and the master-data cut;
   - the schema versions, ingestion runs, transform version and digest;
   - the purpose, the adapter code and a result digest.

   Recomputing at the stored cuts must reproduce the digest.
4. **Completeness.** Flag missing and stale sources, economic conflicts and unresolved master-reference conflicts, and
   reconcile control totals against Core's journal.
5. **Purposes.** Check the purpose against the requester's role and the dataset's permitted purposes, and record denials.
   Exclude parties with an ANALYTICS processing restriction from optional purposes.
6. **Core boundary.** Run under a principal with no Core write path. Core stays the only balance authority.

Register a new adapter as a row in `analytics.adapters` with status `PENDING_SELECTION` until its decision and
qualification evidence exist.
