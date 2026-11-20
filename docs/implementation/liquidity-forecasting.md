# WP-40 liquidity forecasting and controlled finance automation

WP-40 adds migration `0074_liquidity_forecasting.sql` (schema 74). It builds on:
- the WP-24 liquidity observation (signed synthetic evidence, Core commitments);
- the WP-37 governed warehouse (facts at a fixed cut);
- the WP-17 owned cases.

Everything is local and synthetic. Real treasury feeds, real limits and forecast accuracy against real outcomes remain
pending (DEC-03, DEC-06, DEC-07, DEC-12). No accuracy or benefit claim is made beyond the local backtest.

## Inputs and reproducibility

A forecast stores four inputs:

| Input | Source | Rule |
| --- | --- | --- |
| Liquidity observation | WP-24 signed evidence and Core positions | Only `FRESH` evidence against fresh Core positions gives confirmed funds: provider available minus Core outgoing commitments. Stale, missing, invalid or unreachable evidence gives none. |
| Warehouse watermark | `analytics.watermarks` for `ledger_entries` | Lag beyond `maxWarehouseLagSeconds` flags `WAREHOUSE_STALE`. No ingestion yet flags `WAREHOUSE_NEVER_INGESTED`. |
| Flow history | `analytics.liquidity_history` at a recorded fact cut | Daily sum of settlement-account entries for the policy's posting kinds, over the lookback window plus the backtest window. |
| Policy version | `finance.liquidity_policies` | Model, limits, restricted actions, automation and escalation. |

`finance.forecast_compute` is pure. Reproduction recomputes the history at the stored fact cut, recomputes the output and
compares the digest. Each reproduction is recorded. Facts that arrive later have higher sequence numbers and do not
change the result.

## Model and outcome

The model is `TRAILING_MEAN_PLUS_K_SIGMA`, version 1. It uses the last 14 days, needs at least 5 days with data, and
sets k = 2:
- expected outflow = ceil(mean);
- high band = ceil(mean + k × standard deviation);
- projected low = confirmed available − high band.

| Status | When |
| --- | --- |
| `CONFIRMED` | Confirmed funds and no data-quality flags. |
| `UNCERTAIN` | Confirmed funds, but history or warehouse flags (for example `INSUFFICIENT_HISTORY`). |
| `UNCONFIRMED` | No confirmed funds. |

| Outcome | When | Consequence |
| --- | --- | --- |
| `DEGRADED` | Status `UNCONFIRMED` | Owned Finance case (`RESTORE_LIQUIDITY_EVIDENCE`). No restriction. No automated action is possible. |
| `BREACH` | Projected low below `minBufferMinor` | Owned Finance case (`RESTORE_LIQUIDITY_BUFFER`). A restriction on the policy's restricted actions, if none is active. If the status is `CONFIRMED` and automation is on, one prefunding proposal. |
| `WITHIN_LIMITS` | Otherwise | Nothing. A healthy forecast never lifts a restriction by itself. |

An open case of the same kind is reused within an episode. A breach with an active restriction reuses that
restriction's case.

**Backtest.** Every forecast includes a rolling backtest over the previous 7 days, comparing the model's expected outflow
with the actual outflow. The baseline is the previous day's actual. It reports both mean absolute errors, the
high-band coverage, and how many backtest days had data. It is labelled `LOCAL_SYNTHETIC_HISTORY_ONLY`.

**Outcomes against actuals.** The staff view also compares each recorded forecast with the actual outflow of its
business date, once that day has ended.

## Restricted actions

The only restricted action is `EXTERNAL_SUBMISSION`: the payment submission path of WP-20 and WP-39. Payments reads
`finance.liquidity_restricted('EXTERNAL_SUBMISSION')` before reserving. While it is restricted, the payment waits as
`RAIL_PAUSED` with reason `LIQUIDITY_RESTRICTED`, with no hold and nothing sent. Payment recovery resumes it after the
lift.

A lift is maker/checker:
- a Finance maker proposes it;
- a different Finance approver applies it;
- applying needs the latest forecast to be `CONFIRMED`, `WITHIN_LIMITS`, newer than the restriction and within
  `maxForecastAgeSeconds`.

## Automated action: settlement prefunding

| Step | Who | Check |
| --- | --- | --- |
| Propose | The forecast | Confirmed breach. Amount = min(`targetBufferMinor` − projected low, `maxActionMinor`); `ACTION_CAPPED_BY_LIMIT` when capped. |
| Confirm | Finance maker (`finance_adjuster`) | The proposing forecast is still the latest, confirmed, breaching and within the age limit. |
| Approve | Finance approver (`finance_approver`), not the maker | The same forecast checks, re-applied at approval. |
| Book | Core, `core.execute_liquidity_action` | Exact details and digest; both people's live roles; approval under 15 minutes old; per-action and daily limits from the policy version. One `LIQUIDITY_PREFUNDING` batch, keyed by the action id. A replay returns the same batch. |

The posting moves the amount from a seeded synthetic treasury reserve (`…f1d0`, SETTLEMENT kind) to the settlement
clearing account (`…0003`). The provider's position is not assumed to change. Only later signed evidence can confirm it.
A newer forecast supersedes undecided actions, so a stale feed cannot keep an old proposal alive.

## Policies

| Field | Version 1 |
| --- | --- |
| `minBufferMinor` / `targetBufferMinor` | 100,000 / 250,000 (TST 1,000.00 / 2,500.00) |
| `maxActionMinor` / `maxDailyActionMinor` | 5,000,000 / 10,000,000 |
| `maxWarehouseLagSeconds` / `maxForecastAgeSeconds` | 900 / 600 |
| `restrictedActions` | `EXTERNAL_SUBMISSION` |
| `automation.scheduleMinutes` | 0 (forecasts on request only; the worker records forecasts only when a version sets it) |
| `escalation` | `finance-operations`, `CRITICAL` |

A Finance maker proposes a validated document. A different Finance approver applies it.

## Interfaces

| Audience | Endpoints and pages |
| --- | --- |
| Staff | Eleven routes under `/api/v1/staff/liquidity`: forecasts (view, run, read, reproduce), policies (propose, decide), actions (confirm, decide, retry booking), restriction lifts (propose, decide). Staff portal: **Liquidity forecast**. |
| Worker | `finance.forecast_due()` is checked every second; a forecast is recorded only when the active policy schedules it. |

Every step is audited in the `liquidity` stream. The model is
[current-liquidity-forecasting.puml](current-liquidity-forecasting.puml).

## Known limits

- The restriction gates the single-payment external submission path only. The batch rail (WP-36), ATM (WP-35) and
  internal transfers are not gated.
- Forecasts cover one currency (TST) and one horizon day.
- History is settlement outflow only. Inflows and expected provider movements are not modelled in the low band.
- Prefunding is booked in Core. The matching provider-side movement is simulated only by later evidence.

## Not in scope

- Real treasury, central-bank or correspondent feeds; intraday liquidity regulation.
- Machine-learned models; multi-currency and multi-horizon forecasts.
- Automated actions other than settlement prefunding.
