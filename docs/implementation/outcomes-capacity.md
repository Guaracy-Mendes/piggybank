# WP-41 outcome KPIs and capacity (local capacity template)

The user chose a local capacity template on 2026-11-19. WP-41 is planned as production measurement. Production
outcomes, ratified targets and SLAs, cost, and production capacity need DEC-07 (workload and recovery budget) and
DEC-12 (owners and launch authority). None is claimed here. This package delivers three things:
1. KPI definitions computed from local synthetic records (migration `0075_outcome_kpis.sql`, schema 75);
2. one measured capacity change for CF-04;
3. an evidence-based [ADR 0002](../decisions/0002-capacity-and-service-boundaries.md).

## KPI snapshots

`analytics.kpi_snapshot` computes KPI-01..08 for a window of at most 93 days and stores an immutable, digested
snapshot. Data stewards and Finance or product approvers can compute snapshots. Every snapshot is labelled
`LOCAL_SYNTHETIC_NOT_PRODUCTION_OUTCOMES`, with targets `PENDING_DEC-07_DEC-12`. Items whose deadline has not passed
yet are excluded and counted, never counted as failures or successes. A KPI with no denominator is reported as
`NO_DENOMINATOR`, never as met.

| KPI | Numerator / denominator | Exclusions and breakdown |
| --- | --- | --- |
| KPI-01 | Applications decided (approved or declined) within 24 h of first submission / applications first submitted in the window | Excludes applications still inside their 24 h. Breakdown: manual review, abandoned (expired), undecided after 24 h. |
| KPI-02 | Approved with no manual review or information request / approved | Risk-tier segment: `NOT_RECORDED_LOCALLY`. |
| KPI-03 | Opening requests (registry rows) active with no opening revision / opening requests | Breakdown: remediated, not active. |
| KPI-04 | Settled within the SLA / confirmed intents that were valid, funded and approved | Excludes customer, control and funding outcomes (cancelled, expired, blocked, bank-side declines) and intents still within the SLA. SLA 300 s, basis `LOCAL_ASSUMED_PENDING_DEC-04`. Provider rejections and bank failures are counted separately. |
| KPI-05 | Count of unbalanced committed batches plus operations with more than one economic posting | Target 0. Denominator: batches in the window. |
| KPI-06 | Reconciliation differences with an owned case / differences from runs in the window | Deadline `BUSINESS_DAY_DEADLINE_PENDING_DEC-04`. |
| KPI-07 | Not computable from records | `MANUAL_SAMPLE_REQUIRED`. |
| KPI-08 | Decisions with two attributable, distinct people / decisions | Sources: change proposals, routing policy decisions, liquidity policy decisions and liquidity action decisions. |

| Audience | Endpoints and pages |
| --- | --- |
| Staff | `GET` and `POST /api/v1/staff/analytics/kpi-snapshots`. Staff portal: **Outcome KPIs**. |

## Capacity change (CF-04)

**Profile.** WP-30 left the 3x burst failing for payment intents (CF-04). Tracing one intent request showed that
`create_intent` validated the quote in full, then called `challenge()`, which validated the same quote again in the
same request. Each intent therefore paid for:
- three Controls authorization decisions, each appending an audit record and committing;
- five Core authority reads;
- repeated beneficiary, product and quote reads.

**Change.**
- Intent creation now starts the challenge from the quote it has just validated.
- One Core authority read serves both the Controls decision and the delegate limit.

Confirmation still re-validates the quote, authority, beneficiary and limits before any reservation or posting. A
same-key retry still takes the full path. The payments database suite (18 tests) and the full gated pipeline passed.

**Measurement.** All runs used release builds on the same single host, which also ran an unrelated k3s server during
both measurements.

| Run | Parameters | Sustained intent p95 | Burst intents (offered 75/s) | Burst timeouts |
| --- | --- | --- | --- | --- |
| Before (`.local/load/20261119T201637Z`) | 60 s sustained, 300 s burst | 65.9 ms | 61.1/s, p95 5,004 ms | 21,970 |
| After (`.local/load/20261119T203413Z`) | 60 s sustained, 300 s burst | 38.3 ms | 67.9/s, p95 3,795 ms | 0 |
| After, WP-30 parameters (`.local/load/20261119T204944Z`) | 600 s sustained, 300 s burst | 41.9 ms | 64.5/s, p95 4,051 ms, max 4,960 ms | 0 |

In the WP-30-parameter run all 22,500 burst intents returned 201, and the financial invariants held.

**What the measurement does not show.** The burst phase took 349 s to issue 300 s of requests. Every request kind ran
at 86% of the 3x schedule (about 2.6x), because the load generator shares the saturated host. Burst median latencies
(intents 3.6 s) also show queueing. So the 3x criterion is **not demonstrated**, and AT-18 `burst-3x-without-errors`
stays FAIL with these numbers recorded. Demonstrating 3x needs a separate load generator or the production topology
(DEC-07).

## Known limits

- KPI-04 covers single-payment intents only. Batch rail, ATM and internal-only flows are not included.
- KPI-06 counts ownership, not timeliness, until the business-day deadline is ratified.
- No cost model: no production resources exist to price.
