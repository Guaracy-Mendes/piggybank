# WP-37 governed warehouse with one local adapter (`LOCAL_WAREHOUSE`)

WP-37 adds migration `0071_governed_warehouse.sql` (schema 71). It builds a governed warehouse with **one local
storage and processing adapter**, which the user chose on 2026-11-18 as the template for later adapters. No analytical
product is selected: cloud warehouses, lakehouse storage, stream processors and regulatory report formats each need
a decision (DEC-05, DEC-06, DEC-08) and their own qualification. None is claimed here. The adapter contract is in
[infra/analytics/](../../infra/analytics/README.md).

## Boundary

The warehouse lives in a new `analytics` schema. All of it runs through `analytics.*` definer functions called under the
Data module role `bank_data`.

- **Data role privileges.** `bank_data` has EXECUTE on eight API functions, and nothing else in this schema:
  - no table privileges in `analytics`;
  - no usage on the `core` schema, so no Core table and no Core function is reachable.
- **Function bodies.** No function body writes a Core table. The only Core function called is the immutable
  canonical-JSON helper used for the outbox fingerprint. A database test checks both statements on every function.
- **Sources.** The warehouse reads three sources:
  - the Data projections of Core events (`data.transaction_views`, `data.account_views`);
  - Core's immutable journal, for control totals only;
  - master references (`data.master_refs`, `data.master_history`, `data.reference_conflicts`) and WP-28 processing
    restrictions.

Core remains the only balance authority.

## Datasets and schema versions

| Dataset | Source | Economic identity | Economic content | Permitted purposes |
| --- | --- | --- | --- | --- |
| `ledger_entries` | `data.transaction_views` | operation, posting kind, account | amount, currency, business date, original batch | financial reporting, operations analytics |
| `account_states` | `data.account_views` | account, source version | party, product version, state | financial reporting, operations analytics |

Each source declares the fields it can supply (`analytics.source_fields`). Version 1 of each dataset is in
[contracts/events/analytics/](../../contracts/events/analytics/). A data steward can register the next version through
the API. It must be backward compatible:

- the version number must be exactly the previous one plus one;
- every earlier field keeps its type and requiredness;
- new fields are optional;
- every field is one the source supplies with the same type.

Anything else returns 409. Facts record the schema version they were ingested under. An upgrade never rewrites earlier
facts.

## Ingestion

| Mode | Who | Behaviour |
| --- | --- | --- |
| Incremental | Worker, every 30 s | Reads source rows from the dataset watermark up to 30 s ago, at most 5,000 rows per dataset per pass. A pass with nothing new records no run. |
| Backfill | Data steward | Re-reads a source window of up to 93 days. Always recorded as a run, even when nothing is new. Does not move the incremental watermark. |

Every candidate row is validated against the active schema version:
- **Invalid or missing required value:** rejected and recorded once per source row and reason.
- **New economic identity:** a new immutable fact.
- **Same identity and same content:** a duplicate. This is how replay and re-delivery under a new batch id are absorbed.
- **Same identity, different content:** quarantined in `analytics.ingestion_conflicts`. The fact is never overwritten.

A source transaction that commits behind the watermark is not lost silently. The next snapshot reports it as
`STALE_SOURCE`, and a backfill repairs it.

## Transforms and snapshots

`daily_ledger_movement` groups ledger facts by business date, posting kind and canonical party (with internal accounts
grouped as `INTERNAL:<kind>`). For each group it reports the entry count, credits and debits. Version 2 adds the product
version: it uses the schema 2 field when recorded, and otherwise the latest observed account state. Each transform
version carries a digest of its definition.

A snapshot is immutable and records:

| Lineage | Meaning |
| --- | --- |
| `factCut` | Highest fact sequence used. Ingestion and snapshots take the same advisory lock, so the cut is consistent. |
| `masterCut` | Canonical parties are resolved from master history as of this time. |
| `inputs` | Per dataset: the schema versions, the ingestion runs and the fact count. |
| `transformVersion`, `transformDigest`, `adapter`, `purpose` | What produced the snapshot, where, and for which authorized purpose. |
| `resultDigest` | Digest of the ordered rows. Reproduction recomputes it at the stored cuts and records whether it matched. |
| `supersedes`, `explanation` | Previous snapshot for the same transform, purpose and window. The explanation gives the totals delta, new facts, linked corrections and master changes. |

### Completeness and control totals

| Flag | Raised when |
| --- | --- |
| `MISSING_SOURCE` | An input dataset has never had an ingestion run. |
| `STALE_SOURCE` | Projected rows in the window are not yet ingested; the flag gives the count. |
| `MISSING_ACCOUNT_STATE` | A customer account in the window has no observed account state. |
| `ECONOMIC_CONFLICT` | Quarantined conflicts exist for the window. |
| `CONTROL_TOTAL_MISMATCH` | Core's journal entries, credits and debits for the window differ from the warehouse input; the flag gives the differences. |
| `UNRESOLVED_MASTER_CONFLICT` | A party in the output has an open reference conflict. The flag lists the references and states `NO_AUTOMATIC_MERGE`. |

A snapshot is `COMPLETE` only without flags. Control totals compare four stages: Core journal, warehouse input,
exclusions by purpose, and report output. `reconciled` holds when Core equals input and input equals output plus
exclusions.

### Purposes

| Purpose | Roles | Rule |
| --- | --- | --- |
| `FINANCIAL_REPORTING` (mandatory) | data steward, finance adjuster, finance approver | Every fact is included. |
| `OPERATIONS_ANALYTICS` | data steward, data analyst | Excludes parties whose person has a WP-28 `ANALYTICS` restriction applied before the master cut. Also excludes customer accounts whose party cannot be verified. |
| `MARKETING` | data steward, data analyst | No dataset permits it, so every request is denied (audiences are WP-38). |

A role or dataset refusal is recorded in `analytics.access_denials` and audited, and returns 403. Reading, tracing or
reproducing a snapshot re-checks that the reader holds a role for its purpose.

## Corrections and master references

- **Corrections.** A Core correction arrives as a new batch linked to its original, and is ingested as a new fact. The
  next snapshot supersedes the previous one and lists the correction in its explanation. Earlier snapshots and facts are
  unchanged and still reproduce.
- **Master references.** An open reference conflict is flagged, and the party stays separate. After an approved merge
  (the WP-17 maker/checker flow), later snapshots group the party under the new canonical reference. Earlier snapshots
  still reproduce against their own master cut. Core ownership of the account never changes.

## Interfaces

| Audience | Endpoints and pages |
| --- | --- |
| Staff | `GET /api/v1/staff/analytics`, `POST /api/v1/staff/analytics/ingestions` (backfill), `POST /api/v1/staff/analytics/schemas`, `POST /api/v1/staff/analytics/snapshots`, `GET /api/v1/staff/analytics/snapshots/{id}`, `POST .../reproduction`, `GET .../rows/{row}/trace`. Staff portal: **Governed warehouse**, for data stewards and finance staff. |
| Events | `analytics.snapshot-published` v1 (producer `data`, [contract](../../contracts/events/analytics/snapshot-published.v1.schema.json)), with no personal data. |

The ingress safe envelope replaces error bodies, so a denial reason is read from the recorded denials, not from the 403
body. The model is [current-governed-warehouse.puml](current-governed-warehouse.puml).

## Known limits

- Master history uses the transaction start time. A merge whose transaction began before a snapshot's master cut but
  committed after it would make that snapshot fail reproduction. The failure is recorded as a mismatch, not hidden.
- `STALE_SOURCE` counts rejected source rows as uningested until they are corrected at the source.
- Facts and snapshots are kept indefinitely. Analytical retention needs the records schedule (WP-25) extended once an
  adapter is selected.

## Not in scope

- External analytical products, object storage, streaming ingestion and BI tools (DEC-08).
- Regulatory report formats and submissions (DEC-06).
- Audiences, insights and journey analytics (WP-38).
- Schema registration in the staff portal (API only).
