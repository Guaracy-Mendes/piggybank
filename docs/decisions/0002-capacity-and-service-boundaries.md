# ADR 0002 — Capacity work and physical service boundaries (local evidence)

**Date:** 2026-11-19. **Status:** Accepted for local implementation; production topology remains open (DEC-05, DEC-07).
**Parent:** [ADR 0001](0001-local-implementation-baseline.md). **Package:** WP-41, local capacity template.

## Context

WP-30 measured the single-host fixture and left one capacity finding open:
- **CF-04:** payment intents timed out in the 3x burst.

CF-01 to CF-03 had already removed audit-chain and quota hot spots and made the per-module pool size configurable.
WP-41 asks two questions: whether to extract services or change storage, and which optimizations the evidence
supports.

## Evidence

1. **No hot lock remained after CF-01 and CF-02.** During the bursts the host was CPU-bound (including about 1.9 cores
   for rootless port forwarding). Sessions waited on WAL commits and client round trips.
2. **Per-request database work was the controllable cost.** Removing one duplicate validation pass from intent creation
   produced these changes on the same host and parameters (see [outcomes and capacity](../implementation/outcomes-capacity.md)):
   - sustained intent p95 fell from 65.9 to 38.3 ms;
   - burst intents rose from 61.1/s with 21,970 timeouts to 67.9/s with none.
3. **The generator shares the host.** In the WP-30-parameter run every request kind reached the same 86% of the 3x
   schedule. A single host cannot both generate and serve the burst.
4. **Connections are near the budget.** Peak backends were 83 to 88 of PostgreSQL's 100 at pool size 10.

## Decision

- **Keep the modular monolith with module-owned schemas and roles.** No service is extracted now. The bottleneck is
  host CPU and commit round trips, so extraction would add network hops and commits per request. Module boundaries
  (Core money authority, schema ownership, definer-function APIs) already allow later extraction without contract
  changes.
- **Keep PostgreSQL as the single store.** There is no evidence of storage throughput limits. Commit cost is addressed
  by fewer commits per request, not a different store.
- **Optimize by removing redundant work on hot paths,** and only when two conditions hold: a before/after measurement on
  the same host and parameters, and an unchanged control path (confirmation still re-validates everything before money
  moves).
- **Qualify burst capacity with a separate load generator or the production topology.** Single-host results are
  recorded as local evidence only.

## Consequences

- AT-18 burst remains FAIL until 3x is delivered and served. The improvement is recorded with its reports.
- A production topology (DEC-07) needs connection pooling sized to the module count, before any pool increase.
- Revisit extraction when the production workload is ratified and measured per module. Candidates are modules with
  independent scaling needs, such as payments submission or warehouse ingestion. Extraction would keep each module's
  contracts, schema ownership and recovery tests.
