#!/usr/bin/env python3
"""WP-39 routing across two simulated providers, against the real issuer, API, worker and provider simulator.

Proves: a signed capacity non-execution at provider A moves the same instruction (same payment id, digest and Core hold)
to provider B, which settles it and Core books once; a repeat of A's evidence has no effect; an unknown outcome at A
never falls back; an ordinary rejection is final; routing-policy changes need a different approver, and activation needs
shadow evidence. Ledger invariants hold.
"""
import importlib.util
import json
import os
import signal
import subprocess
import sys
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tests/mobile"))
import native_api as n  # noqa: E402

_sec = importlib.util.spec_from_file_location("security_acceptance", ROOT / "tests/acceptance/security/security.py")
security = importlib.util.module_from_spec(_sec)
_sec.loader.exec_module(security)
SIM = "http://127.0.0.1:4182"
RAIL_KEY = "synthetic-rail-authentication-key-only-2026"
REFERENCE = "SIM00000000000000000000000000000001"


def psql(database, statement):
    return subprocess.check_output(["podman", "exec", "piggybank-postgres-1", "psql", "-U", "dev_admin", "-d", database, "-At", "-v", "ON_ERROR_STOP=1", "-c", statement], text=True).strip()


def simulator(path, body):
    status, payload = n.http("POST", SIM + path, body, {"X-Rail-Key": RAIL_KEY})
    assert status in (200, 204), (path, status, payload)
    return payload


def wait_state(alice, payment, wanted, timeout=90):
    seen = []
    until = time.time() + timeout
    while time.time() < until:
        alice.refresh() if n.claims(alice.tokens["access_token"])["exp"] - time.time() < 10 else None
        state = alice.api("GET", "/api/v1/payments/" + payment, expect=200)[1]["state"]
        if not seen or seen[-1] != state:
            seen.append(state)
        if state in wanted:
            return state, seen
        time.sleep(0.5)
    raise AssertionError(f"{payment} never reached {wanted}: {seen}")


def main():
    fixture = subprocess.Popen(["python3", str(ROOT / "tests/browser-customer/server.py")], cwd=ROOT, env=dict(os.environ, BANK_BROWSER_RATE_LIMIT="5000"),
                               start_new_session=True, stdout=subprocess.DEVNULL)
    try:
        for _ in range(240):
            try:
                if fixture.poll() is None and n.http("GET", n.API + "/health/ready")[0] == 200:
                    break
            except OSError:
                pass
            time.sleep(0.5)
        time.sleep(1)
        assert fixture.poll() is None, "fixture exited (ports already bound?)"
        database = json.loads((ROOT / ".local/customer-browser-fixture.json").read_text())["database"]
        alice = n.Session("alice")
        source = n.open_account(alice, "Routingalice")
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), source, "500000"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        ops, ops_csrf = security.workforce_login("reviewer")
        appr, appr_csrf = security.workforce_login("approver")
        reviewer = lambda method, path, body=None: security.staff(ops, ops_csrf, method, path, body)  # noqa: E731
        approver = lambda method, path, body=None: security.staff(appr, appr_csrf, method, path, body)  # noqa: E731
        # The simulated rail starts paused; enable it with the existing maker/checker flow.
        status, rails = reviewer("GET", "/api/v1/staff/rails")
        assert status == 200, status
        if rails["snapshot"].get("paused", True):
            today = time.strftime("%Y-%m-%d", time.gmtime())
            policy = {"id": str(uuid.uuid4()), "rail": "SIMULATED", "version": int(rails["snapshot"].get("version") or 0) + 1, "label": "SYNTHETIC", "enabled": True,
                      "effectiveFrom": today, "effectiveUntil": "2099-12-31", "paymentLimit": {"minorUnits": "100000", "currency": {"code": "TST", "scale": 2}},
                      "calendar": "SYNTHETIC_UTC_24X7"}
            status, proposal = reviewer("POST", "/api/v1/staff/rails", policy)
            assert status == 201, (status, proposal)
            assert approver("POST", f"/api/v1/staff/rails/changes/{proposal['proposalId']}/approval")[0] == 204
            assert reviewer("POST", f"/api/v1/staff/rails/changes/{proposal['proposalId']}/application")[0] == 204
        beneficiary = alice.api("POST", "/api/v1/beneficiaries", {"label": "Simulated rail payee", "receivingReference": REFERENCE, "rail": "SIMULATED"},
                                key=str(uuid.uuid4()), expect=201)[1]["id"]

        def pay(minor, mode=None):
            alice.refresh()
            quote = alice.api("POST", "/api/v1/payment-quotes", {"payerAccountId": source, "beneficiaryId": beneficiary,
                              "amount": {"minorUnits": str(minor), "currency": {"code": "TST", "scale": 2}}, "requestedDate": time.strftime("%Y-%m-%d", time.gmtime())}, expect=201)[1]
            payment = alice.api("POST", "/api/v1/payments", {"quoteId": quote["id"]}, key=str(uuid.uuid4()), expect=201)[1]["id"]
            if mode:
                simulator("/admin/rail-scenarios", {"instructionId": payment, "mode": mode})
            challenge = alice.api("POST", f"/api/v1/payments/{payment}/approval", expect=200)[1]["challengeId"]
            begin = alice.api("POST", f"/api/v1/approvals/{challenge}/native-step-up", expect=200)[1]
            time.sleep(1.1)
            proof = n.native_login("alice", nonce=begin["nonce"], fresh=True)
            assert alice.api("POST", f"/api/v1/approvals/{challenge}/native-proof", {"idToken": proof["id_token"]})[0] == 204
            assert alice.api("POST", f"/api/v1/payments/{payment}/confirmation", {"quoteId": quote["id"]}, expect=202)[1]["id"] == payment
            return payment
        routes = lambda p: psql(database, f"SELECT string_agg(provider,'>' ORDER BY attempt) FROM core.external_routes WHERE operation_id='{p}'")  # noqa: E731
        at_b = lambda p: psql(database, f"SELECT count(*) FROM simulator.rail_b_instructions WHERE id='{p}'")  # noqa: E731
        booked = lambda p: psql(database, f"SELECT count(*) FROM core.journal_batches WHERE operation_id='{p}' AND posting_kind='EXTERNAL_PAYMENT'")  # noqa: E731
        result = {}
        # 1. Capacity non-execution at A: re-routed to B, settled there, booked once.
        first = pay(1500, "REJECT_CAPACITY")
        state, seen = wait_state(alice, first, {"SUBMITTED", "DECLINED", "SETTLED"})
        assert state == "SUBMITTED" and routes(first) == "SYNTHETIC_RAIL>SYNTHETIC_RAIL_B" and at_b(first) == "1", (state, seen, routes(first))
        assert psql(database, f"SELECT (SELECT instruction FROM simulator.rail_instructions WHERE id='{first}')=(SELECT instruction FROM simulator.rail_b_instructions WHERE id='{first}')") == "t"
        settled = simulator("/admin/rail-b-events", {"instructionId": first, "status": "SETTLED"})
        assert n.http("POST", n.API + "/api/v1/providers/simulated/payments", settled)[0] == 202
        state, _ = wait_state(alice, first, {"SETTLED"})
        assert booked(first) == "1"
        result["nonexecution_fallback_same_identity_books_once"] = True
        # 2. Provider A repeating its final (non-execution) evidence has no effect.
        repeat = simulator("/admin/rail-events", {"instructionId": first, "status": "SETTLED"})
        assert repeat["body"]["status"] == "REJECTED"
        assert n.http("POST", n.API + "/api/v1/providers/simulated/payments", repeat)[0] == 202
        assert booked(first) == "1" and alice.api("GET", "/api/v1/payments/" + first, expect=200)[1]["state"] == "SETTLED"
        result["stale_provider_repeat_has_no_effect"] = True
        # 3. Lost response at A: outcome unknown, never re-routed; inquiry resolves at A.
        lost = pay(1600, "LOST_RESPONSE")
        state, seen = wait_state(alice, lost, {"SUBMITTED"}, timeout=120)
        assert "OUTCOME_UNKNOWN" in seen and at_b(lost) == "0" and routes(lost) == "SYNTHETIC_RAIL", (seen, routes(lost))
        result["unknown_outcome_never_falls_back"] = True
        # 4. Ordinary rejection: final, no fallback.
        rejected = pay(1700, "REJECT")
        state, _ = wait_state(alice, rejected, {"DECLINED", "SUBMITTED", "SETTLED"})
        assert state == "DECLINED" and at_b(rejected) == "0" and booked(rejected) == "0"
        result["ordinary_rejection_final"] = True
        # 5. Policy governance: proposer cannot approve; shadow decisions recorded; activation needs shadow evidence.
        weighted = {"strategy": "HEALTH_WEIGHTED", "providers": ["SYNTHETIC_RAIL", "SYNTHETIC_RAIL_B"], "fallbackReasons": ["PROVIDER_CAPACITY_NOT_EXECUTED"], "maxAttempts": 2,
                    "health": {"windowMinutes": 60, "minSamples": 1, "maxUnknownRate": 0.5, "maxNonexecutionRate": 0.5}, "feesMinor": {"SYNTHETIC_RAIL": 0, "SYNTHETIC_RAIL_B": 0},
                    "promotionMinDecisions": 3}
        status, change = reviewer("POST", "/api/v1/staff/routing/changes", {"document": weighted, "mode": "SHADOW"})
        assert status == 200, (status, change)
        assert reviewer("POST", f"/api/v1/staff/routing/changes/{change['changeId']}/decision", {"approve": True})[0] == 403
        assert approver("POST", f"/api/v1/staff/routing/changes/{change['changeId']}/decision", {"approve": True})[0] == 200
        shadowed = pay(1800)
        wait_state(alice, shadowed, {"SUBMITTED"})
        status, view = reviewer("GET", "/api/v1/staff/routing")
        assert view["head"]["shadowVersion"] == change["version"] and view["shadow"]["decisions"] >= 1
        assert any(d["paymentId"] == shadowed and d["shadowProvider"] for d in view["decisions"])
        status, early = reviewer("POST", "/api/v1/staff/routing/changes", {"version": change["version"], "mode": "ACTIVE"})
        assert approver("POST", f"/api/v1/staff/routing/changes/{early['changeId']}/decision", {"approve": True})[0] == 409
        result["policy_change_independent_and_shadowed"] = True
        invariants = psql(database, "SELECT (SELECT count(*) FROM (SELECT batch_id FROM core.journal_entries GROUP BY batch_id HAVING sum(delta::numeric)<>0) u)||'|'||"
                                    "(SELECT count(*) FROM core.ledger_accounts a WHERE a.booked<>coalesce((SELECT sum(delta) FROM core.journal_entries e WHERE e.account_id=a.id),0))")
        assert invariants == "0|0", invariants
        result["ledger_invariants"] = True
        print(json.dumps({"result": "passed", **result}))
    finally:
        if fixture.poll() is None:
            os.killpg(fixture.pid, signal.SIGTERM)
            fixture.wait(timeout=60)


if __name__ == "__main__":
    main()
