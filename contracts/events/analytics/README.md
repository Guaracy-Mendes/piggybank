# Analytics contracts (WP-37)

Version 1 of each governed dataset as registered by migration `0071_governed_warehouse.sql`, and the
`analytics.snapshot-published` event payload. Later dataset versions are registered at runtime through
`POST /api/v1/staff/analytics/schemas` and must be backward compatible: every earlier field is kept with the same type and
requiredness, and new fields are optional and supplied by the source. The database test
`crates/data/tests/cases/analytics.rs` checks that the registered v1 schemas and the event match these files.

These contracts belong to the local warehouse adapter only (user decision 2026-11-18). A selected analytical product
(DEC-05/06/08) must accept the same datasets and lineage fields.
