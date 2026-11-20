# Local R1 qualification (WP-30)

This report records the WP-30 qualification run on 2026-11-17. It covers acceptance, capacity, fault and recovery
results measured on the **local, single-host synthetic environment**. It is not production, provider or launch
qualification: [Not verified here](#not-verified-here) lists what remains open.

Supporting material:
- [Acceptance manifest](../../tests/acceptance/manifest.json) (validated by `make acceptance`)
- [Load suite](../../tests/load/README.md) and [resilience drills](../../tests/resilience/README.md)
- [WP-30 evidence](evidence/WP-30.md)

## Acceptance (FR-OPS-04)

`scripts/acceptance.py` checks that every R1 scenario (AT-01 to AT-20 and AT-24 to AT-30) is present, at branch
level. A `PASS` branch must name evidence that exists in the repository. A `FAIL`, `PENDING` or
`NOT_APPLICABLE_R1` branch must give a reason. Nothing is silently skipped.

| Status | Branches | Meaning |
| --- | --- | --- |
| PASS | 41 | Executable evidence passed in the recorded local pipeline |
| FAIL | 1 | AT-18 3x burst for 5 minutes (capacity finding CF-04) |
| PENDING | 7 | Needs an external decision, provider, deployment or human walkthrough (reason recorded) |
| NOT_APPLICABLE_R1 | 5 | Belongs to an R2 package (WP-33 to WP-38) |

`make acceptance` validates the manifest. `python3 scripts/acceptance.py --strict` exits non-zero while any branch
fails.

## Capacity (NFR-02 to NFR-05)

Command: `python3 tests/load/run.py --sustained 600 --burst 300 --core-seconds 60`. Two measured runs were made: a
baseline, then a second run after the capacity fixes below.

Environment and workload:
- Release binaries, 8 CPU cores, one host running PostgreSQL, Keycloak and NATS in podman (rootless, port-forwarded),
  plus the API, worker, provider simulator and load generator.
- 8 native virtual users with rotating refresh tokens.
- Payers are two hot accounts.

### Final run (after CF-01 and CF-02)

Report: `.local/load/20261118T002830Z/report.json`. Database pool `BANK_DB_POOL_MAX=10`; the peak was 88 backends
against `max_connections` 100.

| Phase | Kind | Requests | Achieved/s | p50 ms | p95 ms | p99 ms | Errors |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Sustained 600 s | balance read | 60,000 | 100.0 | 3.8 | 8.3 | 21.9 | 0 |
| | history read | 60,000 | 100.0 | 8.7 | 18.2 | 47.1 | 0 |
| | quote | 15,001 | 25.0 | 5.7 | 13.0 | 32.1 | 0 |
| | payment intent | 15,001 | 25.0 | 29.4 | 46.9 | 111.5 | 0 |
| Burst 3x, 300 s schedule (325 s with drain) | balance read | 90,000 | 276.9 | 716 | 841 | 897 | 0 |
| | history read | 90,000 | 276.9 | 1,466 | 1,752 | 1,872 | 0 |
| | quote | 22,501 | 69.2 | 790 | 1,080 | 1,189 | 0 |
| | payment intent | 22,501 | 69.2 | 5,003 | 5,004 | 5,006 | 21,769 × 504 |
| Core transfer, 60 s, 16 workers, 200 accounts | transfer | 60,409 | 957.6 | 14.1 | 20.1 | 24.7 | 0 (1,236 idempotent replays) |

### Baseline run (before the fixes)

Report: `.local/load/20261117T225644Z/report.json`. Pool 5.

| Measure | Result |
| --- | --- |
| Sustained phase | Passed: read p95 12.7 ms, command p95 40.0 ms, 0 errors |
| Burst, every kind | Capped at 61% of the offered rate: about 184 reads/s per kind and 46 commands/s |
| Burst payment intents | 22,314 of 22,501 returned 504 |
| Core transfers | 1,303/s, p95 15.4 ms |

### Results

Token refresh errors: none in either run. Financial invariants, checked before and after both runs, all held:
- every journal batch balanced;
- booked balances matched the journal sums;
- no customer account was negative or over-held;
- no economic posting was duplicated.

All 37,502 payment intents in each run were persisted, including the burst requests that returned 504. A 504 here
means the 5 s ingress deadline elapsed, not that the work was lost. Retrying with the same idempotency key returns
the original intent (resilience drill D2).

| NFR check | Result |
| --- | --- |
| NFR-02: reads p95 ≤ 500 ms, p99 ≤ 1,500 ms | **Pass** at the sustained rate |
| NFR-03: commands p95 ≤ 1,000 ms, p99 ≤ 2,500 ms | **Pass** at the sustained rate |
| NFR-04: Core transfer p95 ≤ 2,000 ms | **Pass** (20.1 ms) |
| NFR-05: 200 reads/s and 50 commands/s sustained | **Pass** for 600 s. The one-hour duration was **not run**. |
| NFR-05: 3x burst for 5 minutes without errors | **Fail.** Reads and quotes had no errors, but payment intents topped out at about 69/s against 75/s offered. The queue grew past the deadline. |

### Capacity findings

- **CF-01: single-stream audit-chain serialization (fixed).**
  - *Cause.* `controls.append_audit` locks a stream's head row until commit. Every authenticated read and payment
    appended to one `authorization` stream (21,597 entries in a 60 s burst sample), and every control check
    appended to one `controls` stream.
  - *Effect.* All authenticated traffic was serialized: every kind reached the same 61% of its offered burst rate.
  - *Fix.* Both streams are now split into a fixed set of 16 shard chains (`authorization-NN`, `controls-NN`).
    Authorization records are keyed by their random record id, so a busy account doesn't concentrate on one shard.
    Control checks are keyed by resource, so one resource's checks stay in order. Earlier records stay in the
    legacy chains, and `AuditLog::verify_sharded` verifies the legacy chain and every shard.
  - *Result.* At pool 5, the burst rose from 46 to 63 commands/s.
- **CF-02: quota-counter hot row (fixed).**
  - *Cause.* Every request from one address updates the same per-minute quota row, and it took four round trips
    with a WAL flush while holding that row's lock.
  - *Fix.* It is now one statement, committed with `synchronous_commit` off for that statement. A crash can lose only
    the last few increments of the current minute, which under-counts, never over-counts.
- **CF-03: pool size (configuration).**
  - *Finding.* Once CF-01 was fixed, the five `controls` connections were busy committing.
  - *Change.* `BANK_DB_POOL_MAX` (1–32, default 5) now sets the per-module pool size. At 10, a 30 s burst ran with
    0 errors at about 95% of the offered rate. The 300 s burst still built a payment-intent backlog.
  - *Budget.* The pool size must keep the sum across processes and modules within PostgreSQL `max_connections`.
    The peak here was 88 of 100.
- **CF-04: remaining burst limit is the single host (open).**
  - *Observed.* During the final burst the host was about 85% CPU-busy. Podman's rootless port forwarder alone used
    about 1.9 cores. The `controls` and `core` sessions were waiting on WAL writes and client round trips. No hot
    lock remained.
  - *Why intents.* Creating a payment intent costs several times a quote's database work (authorization, draft,
    control challenge, audit), so intents are the first to exceed the deadline.
  - *Next step.* Qualifying the 3x burst needs either the production topology (DEC-07) or fewer per-intent round
    trips, which is capacity work for WP-41.
  - *WP-41 update (2026-11-19).* Intent creation no longer re-validates the quote it has just validated. With the
    WP-30 parameters all 22,500 burst intents returned 201 (p95 4.05 s; the before run on the same host had 21,970
    timeouts). The host still delivered only 86% of the 3x schedule, so 3x remains unproven and AT-18 burst stays
    FAIL. See [outcomes and capacity](outcomes-capacity.md).
- **CF-05: debug builds (fixed in tooling).** The first smoke run used debug binaries: about 73 reads/s, p95 5 s.
  The load suite now defaults to release builds.

Single-host numbers say nothing about production capacity.

## Fault and recovery

| Drill | Result |
| --- | --- |
| D1: worker SIGKILL while Core commits | 5 events held in the outbox. Drained 2.2 s after restart with 0 duplicate projections. |
| D2: API SIGKILL during 20 concurrent payment commands | Killed with 3 acknowledged and others in flight. All acknowledged commands were queryable after restart, all 20 same-key retries converged, and each quote had at most 1 intent. |
| D3: NATS paused | Commits stayed durable. 5 new projections drained 1.2 s after resume. |
| D4: Keycloak paused | API stayed live. A cached valid token still read (200). Invalid tokens failed closed with the standard error envelope. Sign-in recovered. |
| D5: rail provider SIGKILL before submission | The payment stayed ACCEPTED or CONTROLS_PENDING, never reporting SETTLED. It settled once (1 journal batch) 3.9 s after the provider restarted. |
| Backup and restore with balance reconciliation (WP-29) | `tests/integration/deployment/operations.py`: restored API balances match |
| Lost and timed-out provider responses | Payments and finance database suites (`LOST_RESPONSE`, `BEFORE_ACCEPT_TIMEOUT`) |

The ledger invariants were asserted after every drill.

## Not verified here

- **Production capacity and duration.** The one-hour NFR-05 duration (not run) and any production topology capacity
  (DEC-07).
- **Production recovery.** Multi-zone RPO 0 and regional RTO (NFR-06/07, AT-17). These need the chosen deployment
  (DEC-05, WP-31). All drills here are local process and container faults.
- **Providers.** Real provider behavior, callback key rotation (AT-20) and reconciliation timeliness against a real
  settlement calendar (NFR-14, AT-25) belong to WP-31.
- **Real authentication and key custody.** Real MFA and phishing-resistant factors, and KMS or HSM custody (AT-29,
  DEC-10).
- **Native platforms.** The iOS build and device journey need macOS and Xcode. Store distribution and attestation
  need DEC-10. Only the Android emulator journey was executed.
- **Manual accessibility.** Screen-reader walkthroughs with TalkBack, VoiceOver and NVDA (AT-19).
- **Revocation timing.** A timed measurement of revocation propagation (NFR-09, AT-14).
