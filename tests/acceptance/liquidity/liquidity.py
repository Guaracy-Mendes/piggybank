#!/usr/bin/env python3
"""WP-40 liquidity forecasting and controlled finance automation, against the real issuer, API, worker and simulators.

Proves:
- without signed liquidity evidence a forecast is degraded, shows no confirmed funds and opens an owned Finance case;
- a confirmed breach restricts new external submissions: a customer payment waits as RAIL_PAUSED with no hold and no
  provider instruction;
- the automated prefunding action needs a Finance maker and a different Finance approver, and Core books it once;
- the restriction lifts only by maker/checker on a current within-limits forecast, after which the payment proceeds;
- forecasts reproduce;
- a scheduled policy makes the worker record forecasts on its own.

The isolated runtime has no warehouse history, so the test seeds 21 days of synthetic settlement outflow facts directly.
"""
import importlib.util
import json
import os
import signal
import subprocess
import sys
import time
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tests/mobile"))
import native_api as n  # noqa: E402

_sec = importlib.util.spec_from_file_location("security_acceptance", ROOT / "tests/acceptance/security/security.py")
security = importlib.util.module_from_spec(_sec)
_sec.loader.exec_module(security)
SIM = "http://127.0.0.1:4182"
LIQUIDITY_KEY = "synthetic-local-liquidity-authentication-key-2026"
REFERENCE = "SIM00000000000000000000000000000001"
CLEARING = "4f1a780b-3e31-48ef-a294-000000000003"


def psql(database, statement):
    return subprocess.check_output(["podman", "exec", "piggybank-postgres-1", "psql", "-U", "dev_admin", "-d", database, "-At", "-v", "ON_ERROR_STOP=1", "-c", statement], text=True).strip()


def publish_cash(available):
    now = datetime.now(timezone.utc)
    cash = {"evidenceId": str(uuid.uuid4()), "provider": "SYNTHETIC_LIQUIDITY", "scope": "SYNTHETIC_TST_CLEARING", "currency": {"code": "TST", "scale": 2},
            "version": time.time_ns() // 1000, "asOf": now.isoformat().replace("+00:00", "Z"), "validUntil": (now + timedelta(seconds=60)).isoformat().replace("+00:00", "Z"),
            "settled": str(available), "available": str(available), "reserved": "0", "expectedIncoming": "0", "expectedOutgoing": "0"}
    status, payload = n.http("POST", SIM + "/admin/liquidity-positions", cash, {"X-Liquidity-Key": LIQUIDITY_KEY})
    assert status == 201, (status, payload)


def seed_history(database):
    run = str(uuid.uuid4())
    psql(database, f"INSERT INTO analytics.ingestion_runs VALUES('{run}','ledger_entries',1,'BACKFILL',now()-interval '30 days',now(),21,21,0,0,0,now(),NULL,NULL,now(),now())")
    rows = []
    for offset in range(1, 22):
        day = (datetime.now(timezone.utc).date() - timedelta(days=offset)).isoformat()
        operation = str(uuid.uuid4())
        record = json.dumps({"batchId": str(uuid.uuid4()), "operationId": operation, "postingKind": "EXTERNAL_PAYMENT", "accountId": CLEARING,
                             "accountKind": "SETTLEMENT", "amountMinor": str(10000 + (offset % 3) * 1000), "currency": "TST", "businessDate": day})
        rows.append(f"('ledger_entries',1,'{operation}|EXTERNAL_PAYMENT|{CLEARING}','fixture','{day}','{CLEARING}','{record}'::jsonb,'liquidity-fixture:{offset}',now(),'{run}')")
    psql(database, "INSERT INTO analytics.facts(dataset,schema_version,economic_key,economic_digest,business_date,account_id,record,source_ref,source_recorded_at,run_id) VALUES" + ",".join(rows))


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
        source = n.open_account(alice, "Liquidityalice")
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), source, "500000"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        ops, ops_csrf = security.workforce_login("reviewer")
        appr, appr_csrf = security.workforce_login("approver")
        maker = lambda method, path, body=None: security.staff(ops, ops_csrf, method, path, body)  # noqa: E731
        checker = lambda method, path, body=None: security.staff(appr, appr_csrf, method, path, body)  # noqa: E731
        status, rails = maker("GET", "/api/v1/staff/rails")
        assert status == 200, status
        if rails["snapshot"].get("paused", True):
            today = time.strftime("%Y-%m-%d", time.gmtime())
            policy = {"id": str(uuid.uuid4()), "rail": "SIMULATED", "version": int(rails["snapshot"].get("version") or 0) + 1, "label": "SYNTHETIC", "enabled": True,
                      "effectiveFrom": today, "effectiveUntil": "2099-12-31", "paymentLimit": {"minorUnits": "100000", "currency": {"code": "TST", "scale": 2}},
                      "calendar": "SYNTHETIC_UTC_24X7"}
            status, proposal = maker("POST", "/api/v1/staff/rails", policy)
            assert status == 201, (status, proposal)
            assert checker("POST", f"/api/v1/staff/rails/changes/{proposal['proposalId']}/approval")[0] == 204
            assert maker("POST", f"/api/v1/staff/rails/changes/{proposal['proposalId']}/application")[0] == 204
        beneficiary = alice.api("POST", "/api/v1/beneficiaries", {"label": "Simulated rail payee", "receivingReference": REFERENCE, "rail": "SIMULATED"},
                                key=str(uuid.uuid4()), expect=201)[1]["id"]
        result = {}
        # 1. No signed evidence: degraded, no confirmed funds, owned case, nothing restricted.
        status, degraded = maker("POST", "/api/v1/staff/liquidity/forecasts")
        assert status == 200, (status, degraded)
        assert degraded["outcome"] == "DEGRADED" and degraded["output"]["confirmedAvailableMinor"] is None, degraded
        case = degraded["escalation"]["caseId"]
        assert psql(database, f"SELECT kind||'|'||queue FROM controls.cases WHERE id='{case}'") == "FINANCE|finance-operations"
        assert degraded["restrictionId"] is None and degraded["action"] is None
        result["missing_evidence_degraded_and_escalated"] = True
        # 2. Fresh evidence below the buffer: a confirmed breach restricts, escalates and proposes one action.
        for _ in range(120):
            if psql(database, "SELECT count(*) FROM analytics.watermarks WHERE dataset='ledger_entries'") == "1":
                break
            time.sleep(0.5)
        seed_history(database)
        publish_cash(105000)
        status, breach = maker("POST", "/api/v1/staff/liquidity/forecasts")
        assert status == 200 and breach["status"] == "CONFIRMED" and breach["outcome"] == "BREACH", breach
        assert breach["restrictionId"] and breach["action"]["state"] == "PROPOSED"
        result["confirmed_breach_restricts_and_proposes"] = True

        # 3. A customer payment waits, with no hold and nothing sent to the provider.
        def pay(minor):
            alice.refresh()
            quote = alice.api("POST", "/api/v1/payment-quotes", {"payerAccountId": source, "beneficiaryId": beneficiary,
                              "amount": {"minorUnits": str(minor), "currency": {"code": "TST", "scale": 2}}, "requestedDate": time.strftime("%Y-%m-%d", time.gmtime())}, expect=201)[1]
            payment = alice.api("POST", "/api/v1/payments", {"quoteId": quote["id"]}, key=str(uuid.uuid4()), expect=201)[1]["id"]
            challenge = alice.api("POST", f"/api/v1/payments/{payment}/approval", expect=200)[1]["challengeId"]
            begin = alice.api("POST", f"/api/v1/approvals/{challenge}/native-step-up", expect=200)[1]
            time.sleep(1.1)
            proof = n.native_login("alice", nonce=begin["nonce"], fresh=True)
            assert alice.api("POST", f"/api/v1/approvals/{challenge}/native-proof", {"idToken": proof["id_token"]})[0] == 204
            assert alice.api("POST", f"/api/v1/payments/{payment}/confirmation", {"quoteId": quote["id"]}, expect=202)[1]["id"] == payment
            return payment
        held = pay(1500)
        state, seen = wait_state(alice, held, {"RAIL_PAUSED", "SUBMITTED", "SETTLED"})
        assert state == "RAIL_PAUSED", seen
        time.sleep(3)
        assert psql(database, f"SELECT state||'|'||reason_code FROM payments.intents WHERE id='{held}'") == "RAIL_PAUSED|LIQUIDITY_RESTRICTED"
        assert psql(database, f"SELECT count(*) FROM simulator.rail_instructions WHERE id='{held}'") == "0"
        assert psql(database, f"SELECT count(*) FROM core.holds WHERE operation_id='{held}'") == "0"
        result["breach_stops_external_submission"] = True
        # 4. Automated action: maker confirms; the maker cannot approve; an independent approver approves; Core books once.
        action = breach["action"]["id"]
        assert maker("POST", f"/api/v1/staff/liquidity/actions/{action}/decision", {"approve": True})[0] == 403
        status, confirmed = maker("POST", f"/api/v1/staff/liquidity/actions/{action}/confirmation")
        assert status == 200 and confirmed["state"] == "CONFIRMED", (status, confirmed)
        status, executed = checker("POST", f"/api/v1/staff/liquidity/actions/{action}/decision", {"approve": True})
        assert status == 200 and executed["state"] == "EXECUTED" and executed["created"] is True, (status, executed)
        status, replay = maker("POST", f"/api/v1/staff/liquidity/actions/{action}/execution")
        assert status == 200 and replay["created"] is False and replay["batchId"] == executed["batchId"]
        assert psql(database, f"SELECT count(*) FROM core.journal_batches WHERE operation_id='{action}' AND posting_kind='LIQUIDITY_PREFUNDING'") == "1"
        result["automated_action_independent_and_booked_once"] = True
        # 5. Lift: refused while the latest forecast breaches; applied by the approver on a within-limits forecast.
        status, lift = maker("POST", f"/api/v1/staff/liquidity/restrictions/{breach['restrictionId']}/lifts")
        assert status == 200, (status, lift)
        assert checker("POST", f"/api/v1/staff/liquidity/lifts/{lift['liftId']}/decision", {"approve": True})[0] == 409
        publish_cash(900000)
        status, healthy = maker("POST", "/api/v1/staff/liquidity/forecasts")
        assert status == 200 and healthy["outcome"] == "WITHIN_LIMITS", healthy
        assert maker("POST", f"/api/v1/staff/liquidity/lifts/{lift['liftId']}/decision", {"approve": True})[0] == 403
        status, lifted = checker("POST", f"/api/v1/staff/liquidity/lifts/{lift['liftId']}/decision", {"approve": True})
        assert status == 200 and lifted["outcome"] == "APPLIED" and lifted["restricted"] is False, lifted
        state, seen = wait_state(alice, held, {"SUBMITTED", "SETTLED", "DECLINED"}, timeout=120)
        assert state == "SUBMITTED" and psql(database, f"SELECT count(*) FROM simulator.rail_instructions WHERE id='{held}'") == "1", seen
        result["independent_lift_resumes_payment"] = True
        # 6. Reproduction from stored inputs and the stored fact cut.
        status, reproduction = checker("POST", f"/api/v1/staff/liquidity/forecasts/{breach['id']}/reproduction")
        assert status == 200 and reproduction["matched"] is True, reproduction
        result["forecast_reproduces"] = True
        # 7. A scheduled policy (maker/checker) makes the worker record forecasts on its own.
        status, view = maker("GET", "/api/v1/staff/liquidity/forecasts")
        document = next(p["document"] for p in view["policies"] if p["version"] == view["activeVersion"])
        document["automation"]["scheduleMinutes"] = 1
        status, proposed = maker("POST", "/api/v1/staff/liquidity/policies", {"document": document})
        assert status == 200, (status, proposed)
        assert maker("POST", f"/api/v1/staff/liquidity/policies/{proposed['version']}/decision", {"approve": True})[0] == 403
        assert checker("POST", f"/api/v1/staff/liquidity/policies/{proposed['version']}/decision", {"approve": True})[0] == 200
        until = time.time() + 120
        while time.time() < until:
            if psql(database, f"SELECT count(*) FROM finance.liquidity_forecasts WHERE requested_person IS NULL AND policy_version={proposed['version']}") != "0":
                break
            time.sleep(2)
        else:
            raise AssertionError("worker recorded no scheduled forecast")
        result["scheduled_forecast_by_worker"] = True
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
