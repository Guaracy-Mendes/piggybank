#!/usr/bin/env python3
"""WP-38 audiences, insights and the local journey-partner simulator role, against the real issuer, API and worker.

The test plays the partner (HMAC-signed requests). It proves: an audience includes only members who permit marketing; a
service-outreach audience cannot be exported; exports carry pseudonymous tokens behind a signature; partners can only
send advice (approvals, limits, foreign actions and markup are rejected and recorded, and Core, entitlements and
applications are unchanged); duplicates replay and id reuse conflicts; the customer sees why; withdrawal hides
suggestions, withholds delivery and suppresses new advice without deleting evidence; the outcome report states its
denominators, watermarks and flags.
"""
import hashlib
import hmac
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
KEY = b"synthetic-journey-partner-key-local-only"
PARTNER = "SIM_JOURNEY"


def psql(database, statement):
    return subprocess.check_output(["podman", "exec", "piggybank-postgres-1", "psql", "-U", "dev_admin", "-d", database, "-At", "-v", "ON_ERROR_STOP=1", "-c", statement], text=True).strip()


def partner(method, path, body=None, key=KEY, signed_payload=None):
    raw = json.dumps(body, separators=(",", ":")).encode() if body is not None else None
    payload = signed_payload if signed_payload is not None else raw
    request = n.urllib.request.Request(n.API + path, data=raw, method=method, headers={
        "Content-Type": "application/json", "Piggybank-Journey-Partner": PARTNER,
        "Piggybank-Journey-Signature": hmac.new(key, payload, hashlib.sha256).hexdigest()})
    try:
        with n.urllib.request.urlopen(request, timeout=15) as response:
            data = response.read()
            assert hmac.compare_digest(response.headers["piggybank-journey-signature"], hmac.new(KEY, data, hashlib.sha256).hexdigest())
            return response.status, json.loads(data), data
    except n.urllib.error.HTTPError as error:
        return error.code, None, b""


def fetch_export(export_id, key=KEY):
    path = f"/channels/journeys/v1/exports/{export_id}"
    return partner("GET", path, key=key, signed_payload=f"GET\n{path}\n{PARTNER}".encode())


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
        account = n.open_account(alice, "Journeysalice")
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), account, "50000"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        for _ in range(120):
            if psql(database, f"SELECT count(*) FROM data.transaction_views WHERE account_id='{account}'") == "1":
                break
            time.sleep(0.5)
        alice.api("POST", "/api/v1/notifications/preferences", {"purpose": "MARKETING", "enabled": True}, expect=200)
        ops, csrf = security.workforce_login("reviewer")
        api = lambda method, path, body=None: security.staff(ops, csrf, method, path, body)  # noqa: E731
        window = {"from": (datetime.now(timezone.utc) - timedelta(hours=6)).isoformat(), "to": datetime.now(timezone.utc).isoformat()}
        for dataset in ("account_states", "ledger_entries"):
            assert api("POST", "/api/v1/staff/analytics/ingestions", {"dataset": dataset, **window})[0] == 200
        result = {}
        status, audience = api("POST", "/api/v1/staff/audiences", {"segment": "savings_starter", "version": 1})
        assert status == 200 and audience["members"] >= 1 and audience["purpose"] == "MARKETING", (status, audience)
        member_parties = psql(database, f"SELECT string_agg(party_id::text,',') FROM analytics.audience_members WHERE audience_id='{audience['id']}'")
        consenting = psql(database, "SELECT string_agg(person_id::text,',') FROM data.notification_preferences WHERE purpose='MARKETING' AND enabled")
        assert set(member_parties.split(",")) <= set(consenting.split(",")), "only members who permit marketing"
        result["consented_audience_build"] = True
        status, service = api("POST", "/api/v1/staff/audiences", {"segment": "dormant_check_in", "version": 1})
        assert status == 200
        assert api("POST", f"/api/v1/staff/audiences/{service['id']}/exports", {"partner": PARTNER})[0] == 403
        result["service_outreach_not_exported"] = True
        status, export = api("POST", f"/api/v1/staff/audiences/{audience['id']}/exports", {"partner": PARTNER})
        assert status == 200 and export["exported"] >= 1, export
        assert fetch_export(export["exportId"], key=b"wrong")[0] == 401
        status, delivery, raw = fetch_export(export["exportId"])
        assert status == 200 and account.encode() not in raw and all(t.startswith("m_") for t in delivery["members"])
        token = delivery["members"][0]
        result["export_pseudonymous_signed"] = True
        before = psql(database, "SELECT (SELECT count(*) FROM core.journal_entries)||'|'||(SELECT count(*) FROM core.holds)||'|'||(SELECT count(*) FROM controls.role_grants)||'|'||(SELECT count(*) FROM journeys.applications)")
        advice = {"messageId": "ACC-" + uuid.uuid4().hex[:12], "memberToken": token, "recommendationCode": "BUDGET_REMINDER",
                  "title": "Plan the month ahead", "body": "A short monthly review helps many people. It is always your choice.", "actionRoute": "#/payments"}
        _, accepted, raw1 = partner("POST", "/channels/journeys/v1/recommendations", advice)
        _, _, raw2 = partner("POST", "/channels/journeys/v1/recommendations", advice)
        assert accepted["result"] == "ACCEPTED" and raw1 == raw2
        assert partner("POST", "/channels/journeys/v1/recommendations", dict(advice, title="Other content"))[0] == 409
        assert partner("POST", "/channels/journeys/v1/recommendations", advice, key=b"wrong")[0] == 401
        for extra, reason in [({"approveApplication": True}, "FIELDS_NOT_PERMITTED"), ({"entitlement": "payment_operator"}, "FIELDS_NOT_PERMITTED"),
                              ({"recommendationCode": "APPROVE_CUSTOMER"}, "RECOMMENDATION_NOT_PERMITTED"), ({"actionRoute": "#/roles"}, "ACTION_NOT_PERMITTED"),
                              ({"body": "<img src=x onerror=alert(1)>"}, "CONTENT_NOT_PERMITTED")]:
            _, rejected, _ = partner("POST", "/channels/journeys/v1/recommendations", {**advice, "messageId": "ACC-" + uuid.uuid4().hex[:12], **extra})
            assert rejected["result"] == "REJECTED" and rejected["reason"] == reason, (extra, rejected)
        after = psql(database, "SELECT (SELECT count(*) FROM core.journal_entries)||'|'||(SELECT count(*) FROM core.holds)||'|'||(SELECT count(*) FROM controls.role_grants)||'|'||(SELECT count(*) FROM journeys.applications)")
        assert before == after, (before, after)
        result["partner_advice_only_boundary"] = True
        shown = alice.api("GET", "/api/v1/insights", expect=200)[1]
        partner_items = [i for i in shown["insights"] if i["source"] == "PARTNER"]
        assert partner_items and "partner you allowed marketing" in partner_items[0]["why"] and partner_items[0]["actionRoute"] == "#/payments"
        alice.api("POST", "/api/v1/notifications/preferences", {"purpose": "MARKETING", "enabled": False}, expect=200)
        hidden = alice.api("GET", "/api/v1/insights", expect=200)[1]
        assert hidden["marketingPermitted"] is False and not [i for i in hidden["insights"] if i["purpose"] == "MARKETING"]
        status, withheld, _ = fetch_export(export["exportId"])
        assert status == 200 and token not in withheld["members"] and withheld["withheld"] >= 1
        _, suppressed, _ = partner("POST", "/channels/journeys/v1/recommendations", {**advice, "messageId": "ACC-" + uuid.uuid4().hex[:12]})
        assert suppressed["result"] == "SUPPRESSED"
        assert int(psql(database, "SELECT count(*) FROM analytics.insights WHERE source='PARTNER'")) >= 1, "evidence kept"
        result["withdrawal_hides_and_withholds"] = True
        status, report = api("POST", f"/api/v1/staff/audiences/{audience['id']}/outcome-reports")
        assert status == 200 and report["denominators"]["members"] == audience["members"] and report["denominators"]["withdrawnSinceBuild"] >= 1, report
        assert "ledger_entries" in report["watermarks"] and report["status"] in ("COMPLETE", "INCOMPLETE") and report["outcomes"]["partnerRecommendationsAccepted"] >= 1
        result["outcome_report_denominators_watermarks"] = True
        print(json.dumps({"result": "passed", **result}))
    finally:
        if fixture.poll() is None:
            os.killpg(fixture.pid, signal.SIGTERM)
            fixture.wait(timeout=60)


if __name__ == "__main__":
    main()
