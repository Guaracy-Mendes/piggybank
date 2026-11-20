# Load qualification (WP-30)

`tests/load/run.py` runs a reproducible workload against the isolated local fixture
(`tests/browser-customer/server.py`). The fixture runs the real API, worker, provider simulator, issuer and PostgreSQL
on one host, against a fresh disposable database.

```sh
cargo build --release -p bank-api -p bank-worker -p provider-simulator
python3 tests/load/run.py --sustained 600 --burst 300        # measured run used for WP-30
python3 tests/load/run.py --sustained 3600 --burst 300       # full NFR-05 one-hour objective
make test-load                                               # short smoke (60 s sustained, 30 s burst)
```

| Option | Default | Meaning |
| --- | --- | --- |
| `--read-rps` / `--command-rps` | 200 / 50 | NFR-05 planning workload. A command is one quote plus one payment intent. |
| `--sustained` / `--burst` | 600 / 300 | Phase durations in seconds. The burst runs at 3x the sustained rate. |
| `--users` | 8 | Native (`bank-mobile`) virtual users. Refresh tokens rotate every 20 s. |
| `--core-seconds` | 60 | Concurrent `Core::transfer` phase: 200 accounts, 16 workers, idempotent replays. |
| `--target` | release | Which service binaries to run (`target/<profile>`). Debug builds are not representative. |
| `--pool-max` | 10 | `BANK_DB_POOL_MAX` per module URL for the API and worker. The report records peak backends against PostgreSQL `max_connections` (100 locally). |

## Workload and measurements

- The generator is `bank-load http`, an open-loop scheduler. Latency is measured per request from send to full
  response, with a 10 s client timeout. The API's own 5 s ingress timeout returns 504.
- `achievedRps` counts completed requests divided by the phase's elapsed time, including the drain of in-flight
  requests after the schedule ends. A saturated phase therefore reports fewer requests per second than offered.
- Payers are two funded accounts, alice and bob, each paying the other. Every command contends on the same two
  ledger rows and audit streams, so this is a hot-account worst case, not a uniform spread across customers.
- Every virtual user comes from 127.0.0.1, so the per-IP quota counter is a single row. The fixture raises the
  quota ceiling (`BANK_BROWSER_RATE_LIMIT=1000000`) so the quota itself never rejects requests. The counter's write
  cost is still measured.
- Financial invariants are checked before and after the run:
  - every journal batch balances;
  - booked balances equal the journal sums;
  - no customer account goes negative or holds more than its booked balance;
  - no economic posting is duplicated.

## Report

The report is written to `.local/load/<UTC>/report.json`. It contains the parameters, the per-kind percentiles and
status codes, the token errors, the core phase results, invariants before and after, and named NFR checks. The
temporary config file holds refresh tokens and is deleted as soon as the generator exits.

The report always labels its environment "local single-host fixture (not production capacity)". A run shorter than
3600 s sets `NFR-05 one-hour sustained duration` to false. It never claims the one-hour objective.
