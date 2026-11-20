# WP-39 payment routing and approval optimization (two simulated providers)

WP-39 adds migration `0073_routing_optimization.sql` (schema 73). Following the user decision of 2026-11-18, it is a
**template with two interchangeable simulated providers** for the same rail: `SYNTHETIC_RAIL` (A) and
`SYNTHETIC_RAIL_B` (B). Real providers, their health data and benefit against a real baseline remain pending (DEC-03,
DEC-04, DEC-07, DEC-11). None is claimed here.

## Invariants

1. **One economic identity.** Every attempt sends the identical instruction: the same payment id, input digest, amount
   and Core hold. The hold is kept across a re-route.
2. **Fallback only on proven non-execution.**
   - A payment moves to another provider only after the current provider's signed `REJECTED` evidence with a reason
     Core recognises as non-execution (`PROVIDER_CAPACITY_NOT_EXECUTED`, `PROVIDER_UNAVAILABLE_NOT_EXECUTED`).
   - The routing policy must allow that reason, and the new provider must not have been tried.
   - Unknown outcomes, lost responses and ordinary rejections never re-route.
3. **Core is authoritative.**
   - `core.route_external` keeps an append-only route log. It accepts a re-route only with that proof, verified by Core
     with the provider's key.
   - `core.finalize_external` now accepts final evidence only from the provider of the current route. A payment can
     therefore be booked at most once, from one provider.
4. **Contradictions are quarantined.**
   - If a provider the payment was moved away from repeats its non-execution evidence, it has no effect.
   - Any other claim from that provider (for example a settlement) is quarantined and owned as an exception, and Core
     refuses to book it.
5. **Controls are untouched.**
   - Routing runs after the customer's action-bound approval and the current controls, and reuses them unchanged.
   - Policy documents cannot add providers, widen fallback beyond Core's non-execution reasons, or include any other key
     (for example a step-up exemption).

## Policies, decisions and health

A routing policy version is an immutable document:
- **Strategy:** `PRIMARY_FALLBACK`, which uses providers in order, or `HEALTH_WEIGHTED`, which picks the best measured
  provider above health thresholds.
- **Providers and fallback:** the providers, the fallback reasons and the maximum attempts.
- **Health thresholds:** window, minimum samples, maximum unknown and non-execution rates.
- **Comparison and promotion:** per-provider fees for comparison, and the minimum shadow decisions before activation.

Version 1 (`PRIMARY_FALLBACK`, A then B) is the baseline.

Each payment attempt records a decision (`payments.route_decisions`):
- the provider, policy version, strategy and rule;
- for health-weighted choices, the scores and measured health;
- the shadow policy's choice.

Attempt 1 follows the active policy. Later attempts follow the policy version of attempt 1, so a rollback never changes
an in-flight instruction's fallback rules.

Measured health per provider covers decisions, settled, not executed, rejected and unknown outcomes, and the average
acknowledgement time. It is computed from recorded decisions and attempt-scoped receipts.

## Shadow evaluation, bounded enablement and rollback

| Change | Rule |
| --- | --- |
| Propose | A payment operator proposes a new version (`document`) or a mode for an existing version: `ACTIVE`, `SHADOW` or `SHADOW_OFF`. |
| Decide | A different person with the payment approver role decides. A same-person alias is refused. |
| Shadow | The version is evaluated next to the active policy on every first attempt. Its choice is recorded, never used. |
| Activate | Needs at least `promotionMinDecisions` shadow decisions, unless the version was active before (rollback). |
| Compare | The shadow report gives decisions, the agreement rate, active and shadow choices per provider, fees and measured health. Its baseline is labelled `LOCAL_SIMULATOR_ONLY`. |

## Data model

| Store | Change |
| --- | --- |
| `simulator.rail_b_*` | Provider B's instructions, events and scenarios, separate from A's. The recipient directory is shared. Both providers have a new `REJECT_CAPACITY` scenario. |
| `core.external_routes` | Append-only route attempts with the non-execution proof. |
| `payments.rail_submissions` | `provider` and `route_attempt`. The guard allows only a one-step re-route that resets the provider attempt. |
| `payments.rail_receipts` | `provider` and `route_attempt`. Uniqueness is per attempt. Finality, reporting and recovery read only the current attempt. |
| `payments.route_*` | Policies, head, changes, decisions (immutable except the head). |

## Interfaces

| Audience | Endpoints and pages |
| --- | --- |
| Staff | `GET /api/v1/staff/routing`, `POST /api/v1/staff/routing/changes`, `POST /api/v1/staff/routing/changes/{id}/decision`. Staff portal: **Payment routing**. |
| Providers | The existing callback `POST /api/v1/providers/simulated/payments` verifies evidence with the key of the provider it names. Provider B lives at `/rails-b/…` on the simulator (key `BANK_RAIL_B_SIGNING_KEY`). |

The model is [current-payment-routing.puml](current-payment-routing.puml).

## Known limits

- Finance settlement-statement reconciliation (WP-24) reads provider A's statements only. Provider B settlements are
  booked correctly but not yet matched against a provider B statement.
- Inbound credits and recipient verification use provider A only.
- Health and shadow comparisons use local simulator outcomes, not a real baseline.

## Not in scope

- Real providers, their contracts and health feeds.
- Cost optimization against real fees.
- Cross-currency routing.
