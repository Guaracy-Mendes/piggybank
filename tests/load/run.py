#!/usr/bin/env python3
"""WP-30 reproducible workload on the isolated local fixture (real API, worker, provider, issuer, PostgreSQL).

  python3 tests/load/run.py [--sustained 600] [--burst 300] [--read-rps 200] [--command-rps 50] [--users 8]

NFR-05's planning workload is 200 read/s and 50 command/s sustained for one hour plus a 3x burst for 5 minutes.
The default sustained duration here is shorter. The report records the duration actually run, and a shorter run
never claims the one-hour objective. Financial invariants are checked before and after. Reports go to
.local/load/<UTC>/report.json.
"""
import argparse
import datetime
import json
import os
import signal
import subprocess
import sys
import threading
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests/mobile"))
import native_api as n  # noqa: E402


def psql(database, statement):
    return subprocess.check_output(["podman", "exec", "piggybank-postgres-1", "psql", "-U", "dev_admin", "-d", database, "-At", "-v", "ON_ERROR_STOP=1", "-c", statement], text=True).strip()


def invariants(database):
    return {
        "unbalancedBatches": int(psql(database, "SELECT count(*) FROM (SELECT batch_id FROM core.journal_entries GROUP BY batch_id HAVING sum(delta::numeric)<>0) u")),
        "bookedMismatches": int(psql(database, "SELECT count(*) FROM core.ledger_accounts a WHERE a.booked<>coalesce((SELECT sum(delta) FROM core.journal_entries e WHERE e.account_id=a.id),0)")),
        "negativeCustomer": int(psql(database, "SELECT count(*) FROM core.ledger_accounts WHERE kind='CUSTOMER' AND (booked<0 OR held>booked)")),
        "duplicateEconomicPayments": int(psql(database, "SELECT count(*) FROM (SELECT operation_id,posting_kind FROM core.journal_batches GROUP BY 1,2 HAVING count(*)>1) d")),
        "paymentIntents": int(psql(database, "SELECT count(*) FROM payments.intents")),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sustained", type=int, default=600)
    parser.add_argument("--burst", type=int, default=300)
    parser.add_argument("--read-rps", type=float, default=200)
    parser.add_argument("--command-rps", type=float, default=50)
    parser.add_argument("--users", type=int, default=8)
    parser.add_argument("--core-seconds", type=int, default=60)
    parser.add_argument("--target", choices=["release", "debug"], default="release", help="service binaries under target/<profile>")
    parser.add_argument("--pool-max", type=int, default=10, help="BANK_DB_POOL_MAX for API and worker (PostgreSQL max_connections is 100 locally)")
    args = parser.parse_args()
    out = ROOT / ".local/load" / datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out.mkdir(parents=True)
    env = dict(os.environ, BANK_BROWSER_RATE_LIMIT="1000000", BANK_FIXTURE_TARGET=args.target, BANK_DB_POOL_MAX=str(args.pool_max))
    fixture = subprocess.Popen(["python3", str(ROOT / "tests/browser-customer/server.py")], cwd=ROOT, env=env, start_new_session=True, stdout=subprocess.DEVNULL)
    try:
        for _ in range(240):
            if fixture.poll() is not None:
                raise AssertionError("fixture exited before readiness (is another fixture still bound to its ports?)")
            try:
                if n.http("GET", n.API + "/health/ready")[0] == 200:
                    break
            except OSError:
                pass
            time.sleep(0.5)
        else:
            raise AssertionError("fixture readiness timeout")
        time.sleep(2)
        if fixture.poll() is not None:  # readiness may have been answered by a stale fixture on the same port
            raise AssertionError("fixture exited; refusing to measure a foreign process")
        database = json.loads((ROOT / ".local/customer-browser-fixture.json").read_text())["database"]
        alice, bob = n.Session("alice"), n.Session("bob")
        accounts = {"alice": n.open_account(alice, "Loadpayer"), "bob": n.open_account(bob, "Loadrecipient")}
        core_url = f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"
        for who in ["alice", "bob"]:
            subprocess.run([str(ROOT / "target" / args.target / "bank-fund"), str(uuid.uuid4()), accounts[who], "100000000"], check=True, stdout=subprocess.DEVNULL,
                           env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=core_url))
        beneficiaries = {}
        for payer, payee, session, payee_session in [("alice", "bob", alice, bob), ("bob", "alice", bob, alice)]:
            reference = payee_session.api("POST", f"/api/v1/accounts/{accounts[payee]}/receiving-reference", expect=200)[1]["reference"]
            beneficiaries[payer] = session.api("POST", "/api/v1/beneficiaries", {"label": "Load " + payee, "receivingReference": reference}, key=str(uuid.uuid4()), expect=201)[1]["id"]
        users = []
        for i in range(args.users):
            who = "alice" if i % 2 == 0 else "bob"
            tokens = n.native_login(who)
            users.append({"refreshToken": tokens["refresh_token"], "accountId": accounts[who], "beneficiaryId": beneficiaries[who]})
        before = invariants(database)
        peak = {"connections": 0, "maxConnections": int(psql(database, "SHOW max_connections"))}
        sampling = threading.Event()

        def sample_connections():  # peak backends on the fixture database, against the server-wide budget
            while not sampling.is_set():
                try:
                    peak["connections"] = max(peak["connections"], int(psql(database, f"SELECT count(*) FROM pg_stat_activity WHERE datname='{database}'")))
                except subprocess.CalledProcessError:
                    pass
                sampling.wait(1)
        sampler = threading.Thread(target=sample_connections, daemon=True)
        sampler.start()
        config = out / "config.json"
        config.write_text(json.dumps({"api": n.API, "issuer": n.ISSUER, "users": users, "readRps": args.read_rps, "commandRps": args.command_rps,
                                      "sustainedSeconds": args.sustained, "burstMultiplier": 3.0, "burstSeconds": args.burst, "out": str(out / "http.json")}))
        config.chmod(0o600)
        subprocess.run([str(ROOT / "target" / args.target / "bank-load"), "http", str(config)], check=True, stdout=subprocess.DEVNULL)
        config.unlink()  # contains refresh tokens
        sampling.set()
        sampler.join(timeout=10)
        http_report = json.loads((out / "http.json").read_text())
        core = subprocess.run([str(ROOT / "target" / args.target / "bank-load"), "core", "200", "16", str(args.core_seconds)], check=True, capture_output=True, text=True,
                              env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=core_url,
                                       BANK_LOAD_ADMIN_URL=f"postgres://dev_admin:synthetic-admin-only@127.0.0.1:15432/{database}"))
        core_report = json.loads(core.stdout.strip().splitlines()[-1])
        after = invariants(database)
        sustained, burst = http_report["phases"]
        reads = [sustained["kinds"].get(k, {}) for k in ("read_balance", "read_history")]
        commands = [sustained["kinds"].get(k, {}) for k in ("command_quote", "command_payment_intent")]
        checks = {
            "NFR-02 reads p95<=500ms": max(r.get("p95Ms", 0) for r in reads) <= 500,
            "NFR-02 reads p99<=1500ms": max(r.get("p99Ms", 0) for r in reads) <= 1500,
            "NFR-03 commands p95<=1000ms": max(c.get("p95Ms", 0) for c in commands) <= 1000,
            "NFR-03 commands p99<=2500ms": max(c.get("p99Ms", 0) for c in commands) <= 2500,
            "NFR-04 core transfer p95<=2000ms": core_report["p95Ms"] <= 2000,
            "NFR-05 sustained read rate achieved": sum(r.get("achievedRps", 0) for r in reads) >= 0.95 * args.read_rps,
            "NFR-05 sustained command rate achieved": sum(c.get("achievedRps", 0) for c in commands) >= 0.95 * args.command_rps,
            "NFR-05 one-hour sustained duration": args.sustained >= 3600,
            "Burst 3x without 5xx or timeouts": all(k.get("failures", 0) == 0 for k in burst["kinds"].values()),
            "Invariants hold after load": after["unbalancedBatches"] == 0 and after["bookedMismatches"] == 0 and after["negativeCustomer"] == 0 and after["duplicateEconomicPayments"] == 0,
        }
        report = {"generatedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(), "environment": "local single-host fixture (not production capacity)",
                  "parameters": vars(args), "databaseConnections": peak, "invariantsBefore": before, "invariantsAfter": after, "http": http_report, "core": core_report, "checks": checks}
        (out / "report.json").write_text(json.dumps(report, indent=2))
        print(json.dumps({"report": str(out.relative_to(ROOT) / "report.json"), "checks": checks, "databaseConnections": peak,
                          "sustained": {k: {m: round(v[m], 1) for m in ("achievedRps", "p95Ms", "p99Ms")} for k, v in sustained["kinds"].items()},
                          "burst": {k: {m: round(v[m], 1) for m in ("achievedRps", "p95Ms", "p99Ms")} | {"failures": v["failures"]} for k, v in burst["kinds"].items()},
                          "core": {k: core_report[k] for k in ("committedPerSecond", "p95Ms", "p99Ms", "idempotentReplays")}}))
        if not checks["Invariants hold after load"]:
            raise SystemExit("financial invariants violated after load: " + json.dumps(after))
    finally:
        if fixture.poll() is None:
            os.killpg(fixture.pid, signal.SIGTERM)
            fixture.wait(timeout=60)


if __name__ == "__main__":
    main()
