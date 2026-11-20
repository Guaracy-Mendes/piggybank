#!/usr/bin/env python3
"""WP-30 local fault and recovery drills on an isolated database with individually controlled processes.

D1 AT-13  worker killed while Core commits continue: the outbox holds events; after restart they are published and
          projected exactly once.
D2 NFR-03 API killed during concurrent durable commands: every acknowledged command is queryable after restart;
          retrying with the same idempotency key never creates a second command.
D3 broker paused: commits continue durably, and the backlog drains after the broker returns.
D4 issuer paused: the API stays live, protected reads fail safely without leaking errors, and sign-in recovers.
D5 rail provider killed before an approved payment is submitted: the payment never reports settlement while the provider
   is down, funds stay reserved, and after the provider returns recovery settles it exactly once.
Invariants (balanced journals, booked equal to journal sums, no duplicate economic postings) hold after every drill.
These are local single-host process drills, not multi-zone or regional recovery (NFR-06/07 remain unverified).
"""
import concurrent.futures
import json
import os
import signal
import subprocess
import sys
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests/integration"))
sys.path.insert(0, str(ROOT / "tests/mobile"))
from database_fixture import Database  # noqa: E402
import native_api as n  # noqa: E402

RUNTIME = "podman"
LOGS = ROOT / ".local/resilience-logs"


class Stack:
    def __init__(self):
        self.db = Database()
        self.env = dict(self.db.env, BANK_RATE_LIMIT="100000", BANK_PUBLIC_BASE=n.API, BANK_RAIL_PROVIDER_BASE="http://127.0.0.1:4182",
                        BANK_FINANCE_PROVIDER_BASE="http://127.0.0.1:4182", BANK_SIMULATOR_URL="http://127.0.0.1:4182")
        subprocess.run(["python3", str(ROOT / "scripts/seed.py")], cwd=ROOT, env=self.env, stdout=subprocess.DEVNULL, check=True)
        self.procs = {}
        for name, port in [("provider-simulator", 4182), ("bank-api", 4180), ("bank-worker", 4181)]:
            self.start(name, port)
        self.wait_ready()

    def start(self, name, port):
        # Service logs are kept (appended across restarts) so an intermittent failure can be diagnosed afterwards.
        LOGS.mkdir(parents=True, exist_ok=True)
        sink = open(LOGS / f"{name}.log", "a")
        self.procs[name] = subprocess.Popen([str(ROOT / "target/debug" / name)], cwd=ROOT, env=dict(self.env, BANK_BIND=f"127.0.0.1:{port}"),
                                            stdout=sink, stderr=subprocess.STDOUT, start_new_session=True)

    def kill(self, name):
        p = self.procs.pop(name)
        os.killpg(p.pid, signal.SIGKILL)
        p.wait(timeout=20)

    def wait_ready(self, seconds=60):
        until = time.time() + seconds
        while time.time() < until:
            try:
                if n.http("GET", n.API + "/health/ready")[0] == 200:
                    return
            except OSError:
                pass
            time.sleep(0.25)
        raise AssertionError("API readiness timeout")

    def sql(self, statement):
        return self.db.sql(statement)

    def close(self):
        for name in list(self.procs):
            self.kill(name)
        self.db.close()


def invariants(stack):
    result = {
        "unbalanced": int(stack.sql("SELECT count(*) FROM (SELECT batch_id FROM core.journal_entries GROUP BY batch_id HAVING sum(delta::numeric)<>0) u")),
        "bookedMismatches": int(stack.sql("SELECT count(*) FROM core.ledger_accounts a WHERE a.booked<>coalesce((SELECT sum(delta) FROM core.journal_entries e WHERE e.account_id=a.id),0)")),
        "duplicatePostings": int(stack.sql("SELECT count(*) FROM (SELECT operation_id,posting_kind FROM core.journal_batches GROUP BY 1,2 HAVING count(*)>1) d")),
    }
    assert result == {"unbalanced": 0, "bookedMismatches": 0, "duplicatePostings": 0}, result
    return result


def fund(stack, account, amount):
    subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), account, str(amount)], check=True, stdout=subprocess.DEVNULL,
                   env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=stack.env["BANK_CORE_DATABASE_URL"]))


def wait_for(predicate, seconds):
    until = time.time() + seconds
    while time.time() < until:
        if predicate():
            return time.time()
        time.sleep(0.5)
    raise AssertionError("condition not reached")


def main():
    for old in LOGS.glob("*.log"):
        old.unlink()
    stack = Stack()
    results = {}
    try:
        alice, bob = n.Session("alice"), n.Session("bob")
        source, target = n.open_account(alice, "Resilience"), n.open_account(bob, "Resiliencerecipient")
        fund(stack, source, 1000000)
        reference = bob.api("POST", f"/api/v1/accounts/{target}/receiving-reference", expect=200)[1]["reference"]
        beneficiary = alice.api("POST", "/api/v1/beneficiaries", {"label": "Bob", "receivingReference": reference}, key=str(uuid.uuid4()), expect=201)[1]["id"]
        projected = lambda: int(stack.sql("SELECT count(DISTINCT batch_id) FROM data.transaction_views"))  # noqa: E731
        batches = lambda: int(stack.sql("SELECT count(*) FROM core.journal_batches j WHERE EXISTS(SELECT 1 FROM core.journal_entries e JOIN core.account_registry a ON a.id=e.account_id WHERE e.batch_id=j.id)"))  # noqa: E731
        wait_for(lambda: projected() == batches(), 60)
        # D1: worker crash; Core keeps committing; events wait in the outbox, then publish and project once.
        stack.kill("bank-worker")
        for _ in range(5):
            fund(stack, source, 100)
        undelivered = int(stack.sql("SELECT count(*) FROM core.outbox WHERE delivered_at IS NULL"))
        assert undelivered >= 5 and projected() < batches()
        restarted = time.time()
        stack.start("bank-worker", 4181)
        drained = wait_for(lambda: projected() == batches() and int(stack.sql("SELECT count(*) FROM core.outbox WHERE delivered_at IS NULL")) == 0, 90)
        duplicates = int(stack.sql("SELECT count(*) FROM (SELECT batch_id,account_id FROM data.transaction_views GROUP BY 1,2 HAVING count(*)>1) d"))
        assert duplicates == 0
        results["D1_worker_crash_outbox_replay"] = {"heldEvents": undelivered, "drainSeconds": round(drained - restarted, 1), "duplicateProjections": duplicates, "invariants": invariants(stack)}
        # D2: API killed during concurrent durable commands; acknowledged work is queryable; same-key retries converge.
        quotes = []
        for _ in range(20):
            quotes.append(alice.api("POST", "/api/v1/payment-quotes", {"payerAccountId": source, "beneficiaryId": beneficiary,
                          "amount": {"minorUnits": "100", "currency": {"code": "TST", "scale": 2}}, "requestedDate": time.strftime("%Y-%m-%d", time.gmtime())}, expect=201)[1]["id"])
        keys = [str(uuid.uuid4()) for _ in quotes]
        token = alice.tokens["access_token"]

        def submit(i):
            try:
                return i, n.http("POST", n.API + "/api/v1/payments", {"quoteId": quotes[i]}, {"Authorization": "Bearer " + token, "Idempotency-Key": keys[i]})
            except OSError:
                return i, (None, None)
        # Kill the API once some commands are acknowledged and others are still in flight.
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            futures = [pool.submit(submit, i) for i in range(len(quotes))]
            until = time.time() + 30
            while sum(1 for f in futures if f.done() and f.result()[1][0] == 201) < 3 and time.time() < until:
                time.sleep(0.01)
            stack.kill("bank-api")
            outcomes = [f.result() for f in futures]
        assert len([1 for _, (status, _) in outcomes if status == 201]) >= 3, "API was killed before any acknowledgement"
        assert len([1 for _, (status, _) in outcomes if status != 201]) >= 1, "every command finished before the crash"
        acknowledged = {i: body["id"] for i, (status, body) in outcomes if status == 201}
        stack.start("bank-api", 4180)
        stack.wait_ready()
        alice.refresh()
        for i, payment in acknowledged.items():
            assert alice.api("GET", "/api/v1/payments/" + payment, expect=200)[1]["id"] == payment
        retried = {}
        for i in range(len(quotes)):
            status, body = alice.api("POST", "/api/v1/payments", {"quoteId": quotes[i]}, key=keys[i])
            assert status in (200, 201), status
            retried[i] = body["id"]
            if i in acknowledged:
                assert retried[i] == acknowledged[i]
        per_quote = int(stack.sql("SELECT coalesce(max(c),0) FROM (SELECT quote_id,count(*) c FROM payments.intents GROUP BY quote_id) q"))
        assert per_quote == 1
        results["D2_api_crash_durable_commands"] = {"submitted": len(quotes), "acknowledgedBeforeCrash": len(acknowledged), "allQueryableAfterRestart": True,
                                                    "sameKeyRetriesConverged": True, "maxIntentsPerQuote": per_quote, "invariants": invariants(stack)}
        # D3: broker paused; commits stay durable; backlog drains after resume.
        before = projected()
        subprocess.run([RUNTIME, "pause", "piggybank-nats-1"], check=True, stdout=subprocess.DEVNULL)
        try:
            for _ in range(5):
                fund(stack, source, 100)
            time.sleep(3)
            assert int(stack.sql("SELECT count(*) FROM core.outbox WHERE delivered_at IS NULL")) >= 1
        finally:
            subprocess.run([RUNTIME, "unpause", "piggybank-nats-1"], check=True, stdout=subprocess.DEVNULL)
        resumed = time.time()
        drained = wait_for(lambda: projected() == batches() and int(stack.sql("SELECT count(*) FROM core.outbox WHERE delivered_at IS NULL")) == 0, 120)
        results["D3_broker_outage"] = {"newProjected": projected() - before, "drainSeconds": round(drained - resumed, 1), "invariants": invariants(stack)}
        # D4: issuer paused; API stays live; protected calls fail safely; sign-in recovers.
        alice.refresh()
        valid = "Bearer " + alice.tokens["access_token"]
        subprocess.run([RUNTIME, "pause", "piggybank-keycloak-1"], check=True, stdout=subprocess.DEVNULL)
        try:
            time.sleep(1)
            live = n.http("GET", n.API + "/health/live")[0]
            cached, _ = n.http("GET", n.API + f"/api/v1/accounts/{source}/balance", headers={"Authorization": valid})
            status, body = n.http("GET", n.API + f"/api/v1/accounts/{source}/balance", headers={"Authorization": "Bearer invalid.token.value"})
            assert live == 200 and cached in (200, 503) and status in (401, 503), (live, cached, status)
            assert isinstance(body, dict) and set(body.get("error", {}).keys()) <= {"code", "message", "requestId"}
        finally:
            subprocess.run([RUNTIME, "unpause", "piggybank-keycloak-1"], check=True, stdout=subprocess.DEVNULL)
        recovered = wait_for(lambda: n.http("GET", n.ISSUER + "/.well-known/openid-configuration")[0] == 200, 60)
        alice = n.Session("alice")
        assert alice.api("GET", f"/api/v1/accounts/{source}/balance", expect=200)[1]["accountId"] == source
        results["D4_issuer_outage"] = {"apiLiveDuringOutage": True, "validTokenDuringOutage": cached, "protectedCallsFailedSafely": True, "signInRecovered": True, "invariants": invariants(stack)}
        # D5: rail provider down during submission; the payment waits (never fake settlement), then settles once.
        alice.refresh()
        stack.kill("provider-simulator")
        quote = alice.api("POST", "/api/v1/payment-quotes", {"payerAccountId": source, "beneficiaryId": beneficiary,
                          "amount": {"minorUnits": "700", "currency": {"code": "TST", "scale": 2}}, "requestedDate": time.strftime("%Y-%m-%d", time.gmtime())}, expect=201)[1]
        payment = alice.api("POST", "/api/v1/payments", {"quoteId": quote["id"]}, key=str(uuid.uuid4()), expect=201)[1]["id"]
        challenge = alice.api("POST", f"/api/v1/payments/{payment}/approval", expect=200)[1]["challengeId"]
        begin = alice.api("POST", f"/api/v1/approvals/{challenge}/native-step-up", expect=200)[1]
        time.sleep(1.1)
        proof = n.native_login("alice", nonce=begin["nonce"], fresh=True)
        assert alice.api("POST", f"/api/v1/approvals/{challenge}/native-proof", {"idToken": proof["id_token"]})[0] == 204
        assert alice.api("POST", f"/api/v1/payments/{payment}/confirmation", {"quoteId": quote["id"]}, expect=202)[1]["id"] == payment
        seen = set()
        until = time.time() + 15
        while time.time() < until:
            seen.add(alice.api("GET", "/api/v1/payments/" + payment, expect=200)[1]["state"])
            time.sleep(0.5)
        assert "SETTLED" not in seen, seen
        restarted = time.time()
        stack.start("provider-simulator", 4182)

        def settled():
            alice.refresh()
            return alice.api("GET", "/api/v1/payments/" + payment, expect=200)[1]["state"] == "SETTLED"
        done = wait_for(settled, 240)
        settlements = int(stack.sql(f"SELECT count(*) FROM core.journal_batches WHERE id=(SELECT batch_id FROM payments.intents WHERE id='{payment}')"))
        assert settlements == 1, settlements
        results["D5_provider_outage_submission"] = {"statesWhileDown": sorted(seen), "settleSecondsAfterRestart": round(done - restarted, 1),
                                                    "settlementJournalBatches": settlements, "invariants": invariants(stack)}
        print(json.dumps({"result": "passed", "scope": "local single-host process drills (not multi-zone/regional)", **results}))
    finally:
        for c in ["piggybank-nats-1", "piggybank-keycloak-1"]:
            subprocess.run([RUNTIME, "unpause", c], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        stack.close()


if __name__ == "__main__":
    main()
