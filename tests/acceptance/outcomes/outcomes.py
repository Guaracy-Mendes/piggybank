#!/usr/bin/env python3
"""WP-41 outcome KPIs (local capacity template), against the real issuer, API, worker and rail simulator.

The test onboards a customer, settles one external payment and has one rejected by the provider. It then computes a
staff KPI snapshot through the API and checks that each KPI states its numerator, denominator and exclusions, that the
provider failure is reported separately from bank failures, and that the snapshot is labelled local synthetic.
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
RAIL_KEY = "synthetic-rail-authentication-key-only-2026"
REFERENCE = "SIM00000000000000000000000000000001"


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
        started = datetime.now(timezone.utc) - timedelta(minutes=5)
        alice = n.Session("alice")
        source = n.open_account(alice, "Outcomesalice")
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), source, "500000"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        ops, ops_csrf = security.workforce_login("reviewer")
        appr, appr_csrf = security.workforce_login("approver")
        reviewer = lambda method, path, body=None: security.staff(ops, ops_csrf, method, path, body)  # noqa: E731
        approver = lambda method, path, body=None: security.staff(appr, appr_csrf, method, path, body)  # noqa: E731
        status, rails = reviewer("GET", "/api/v1/staff/rails")
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
        settled = pay(1200)
        wait_state(alice, settled, {"SUBMITTED"})
        evidence = simulator("/admin/rail-events", {"instructionId": settled, "status": "SETTLED"})
        assert n.http("POST", n.API + "/api/v1/providers/simulated/payments", evidence)[0] == 202
        wait_state(alice, settled, {"SETTLED"})
        rejected = pay(1300, "REJECT")
        state, _ = wait_state(alice, rejected, {"DECLINED", "SETTLED"})
        assert state == "DECLINED"
        result = {}
        window = {"from": started.isoformat().replace("+00:00", "Z"), "to": (datetime.now(timezone.utc) + timedelta(minutes=1)).isoformat().replace("+00:00", "Z")}
        status, snapshot = approver("POST", "/api/v1/staff/analytics/kpi-snapshots", window)
        assert status == 200, (status, snapshot)
        kpis = {k["code"]: k for k in snapshot["kpis"]}
        assert snapshot["evidence"] == "LOCAL_SYNTHETIC_NOT_PRODUCTION_OUTCOMES" and len(kpis) == 8
        result["snapshot_labelled_local"] = True
        assert kpis["KPI-03"]["denominator"] >= 1 and kpis["KPI-03"]["numerator"] == kpis["KPI-03"]["denominator"], kpis["KPI-03"]
        assert kpis["KPI-01"]["denominator"] + kpis["KPI-01"]["exclusions"]["immatureUnder24h"] >= 1, kpis["KPI-01"]
        result["onboarding_and_opening_counted"] = True
        k4 = kpis["KPI-04"]
        assert k4["denominator"] == 2 and k4["numerator"] == 1, k4
        assert k4["breakdown"] == {"providerFailures": 1, "bankFailures": 0}, k4
        assert k4["slaBasis"] == "LOCAL_ASSUMED_PENDING_DEC-04"
        result["payment_success_separates_provider_failures"] = True
        assert kpis["KPI-05"]["value"] == "0" and kpis["KPI-05"]["meetsTarget"] is True
        assert kpis["KPI-07"]["status"] == "MANUAL_SAMPLE_REQUIRED"
        result["integrity_zero_and_manual_kpi_not_claimed"] = True
        status, listing = reviewer("GET", "/api/v1/staff/analytics/kpi-snapshots")
        assert status == 200 and listing[0]["digest"] == snapshot["digest"]
        result["snapshot_listed_with_digest"] = True
        print(json.dumps({"result": "passed", **result}))
    finally:
        if fixture.poll() is None:
            os.killpg(fixture.pid, signal.SIGTERM)
            fixture.wait(timeout=60)


if __name__ == "__main__":
    main()
