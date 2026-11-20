# WP-29 deployment and operations acceptance manifest

| Branch | Executable evidence |
| --- | --- |
| Configuration failure: production profile, malformed key ring, world-readable, short or missing secret files refuse startup (API and worker) | `config_failure.py` |
| Metrics require a token; HTTP histograms and queue, invariant and alert gauges exposed | `config_failure.py` |
| NFR-10 / AT-20: projection gap pages the primary within 60 s via the worker; the paged person acknowledges; escalation to the secondary | `operations.py`; `crates/data/tests/cases/operations.rs` `critical_alerts_page_the_primary_escalate_to_the_secondary_and_need_the_paged_person` |
| FR-OPS-04 / AT-17 (local): backup and restore with invariants, chain heads and balance verified before writes | `operations.py` |
| NFR-12: released migrations immutable; new migrations expand-only | `scripts/migrations.py check` |
| Local CI-equivalent pipeline with sanitized evidence | `scripts/ci.py` (`make ci`) |
| Images, Compose host boundary and sealed archive | `make cloud-check` (existing `tests/deployment/check.py`) |

Pending, and recorded explicitly:
- Remote CI execution (no host chosen).
- Production RPO/RTO, failover and regional restore (WP-30).
- A real pager and staffed rota (DEC-12).
