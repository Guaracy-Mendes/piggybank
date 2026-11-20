#!/usr/bin/env python3
"""WP-34 assisted-channel acceptance (FR-CH-03, FR-CH-04, AT-22) against the real local issuer, API and worker.

A call-center agent verifies the customer, opens an assisted session and waits for the customer to authorize it in the
app. The agent then reads the customer's accounts and applies a protective freeze; the same restriction blocks the
customer's own payment. Payments are not enabled in any channel (DEC-09). When the customer ends the session the agent
is refused at once. Audit records name the staff actor, the represented customer and the channel.
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


def psql(database, statement):
    return subprocess.check_output(["podman", "exec", "piggybank-postgres-1", "psql", "-U", "dev_admin", "-d", database, "-At", "-v", "ON_ERROR_STOP=1", "-c", statement], text=True).strip()


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
        alice, bob = n.Session("alice"), n.Session("bob")
        account = n.open_account(alice, "Assistedalice")
        target = n.open_account(bob, "Assistedbob")
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), account, "100000"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        person = psql(database, f"SELECT person_id FROM core.account_relationships WHERE account_id='{account}' AND kind='OWNER'")
        agent, csrf = security.workforce_login("reviewer")
        status, opened = security.staff(agent, csrf, "POST", "/api/v1/staff/assisted", {"channel": "CALL_CENTER", "customerPerson": person,
                                        "verification": "ID_DOCUMENT_SIGHTED", "purpose": "Caller reports a lost card"}, key=str(uuid.uuid4()))
        assert status == 400, ("verification not permitted for the channel", status)
        status, opened = security.staff(agent, csrf, "POST", "/api/v1/staff/assisted", {"channel": "CALL_CENTER", "customerPerson": person,
                                        "verification": "VERIFIED_CONTACT_CALLBACK", "purpose": "Caller reports a lost card"}, key=str(uuid.uuid4()))
        assert status == 201 and opened["state"] == "AWAITING_CUSTOMER" and "DRAFT_PAYMENT" not in opened["operations"], (status, opened)
        session = opened["id"]
        assert security.staff(agent, csrf, "GET", f"/api/v1/staff/assisted/{session}/accounts")[0] == 403, "read before customer authorization"
        assert bob.api("POST", f"/api/v1/assisted/{session}/authorization", {"approve": True})[0] == 404
        pending = [s for s in alice.api("GET", "/api/v1/assisted", expect=200)[1] if s["id"] == session]
        assert pending and pending[0]["channel"] == "CALL_CENTER" and "staffPerson" not in pending[0]
        assert alice.api("POST", f"/api/v1/assisted/{session}/authorization", {"approve": True}, expect=200)[1]["state"] == "AUTHORIZED"
        status, listed = security.staff(agent, csrf, "GET", f"/api/v1/staff/assisted/{session}/accounts")
        assert status == 200 and [a["account"]["id"] for a in listed["accounts"]] == [account], listed
        assert security.staff(agent, csrf, "POST", f"/api/v1/staff/assisted/{session}/freeze", {"accountId": target})[0] == 404, "other customer's account"
        assert security.staff(agent, csrf, "POST", f"/api/v1/staff/assisted/{session}/freeze", {"accountId": account})[0] == 204
        # The protective restriction applies to the customer's own channel too (common rules).
        reference = bob.api("POST", f"/api/v1/accounts/{target}/receiving-reference", expect=200)[1]["reference"]
        recipient = alice.api("POST", "/api/v1/beneficiaries", {"label": "Bob assisted", "receivingReference": reference}, key=str(uuid.uuid4()), expect=201)[1]["id"]
        quote = alice.api("POST", "/api/v1/payment-quotes", {"payerAccountId": account, "beneficiaryId": recipient,
                          "amount": {"minorUnits": "100", "currency": {"code": "TST", "scale": 2}}, "requestedDate": time.strftime("%Y-%m-%d", time.gmtime())})
        assert quote[0] in (404, 409), quote
        assert psql(database, f"SELECT debit_blocked FROM core.ledger_accounts WHERE id='{account}'") == "t"
        # The customer ends the session; the agent is refused immediately.
        assert alice.api("POST", f"/api/v1/assisted/{session}/closure", expect=200)[1]["state"] == "CLOSED"
        assert security.staff(agent, csrf, "GET", f"/api/v1/staff/assisted/{session}/accounts")[0] == 403
        # Audit identifies staff actor, represented customer and channel.
        staff_subject = psql(database, "SELECT subject FROM controls.person_links l JOIN controls.role_grants g ON g.person_id=l.person_id WHERE g.role='assisted_service' LIMIT 1")
        traced = int(psql(database, f"SELECT count(*) FROM controls.audit_events WHERE stream LIKE 'authorization-%' AND payload->>'actorSubject'='{staff_subject}' AND payload->>'representedParty'='{person}'"))
        decisions = psql(database, f"SELECT string_agg(DISTINCT reason,',' ORDER BY reason) FROM controls.assisted_actions WHERE session_id='{session}'")
        assert traced >= 2 and "CHANNEL_POLICY" in decisions and "SESSION_NOT_ACTIVE" in decisions, (traced, decisions)
        status, view = security.staff(agent, csrf, "GET", "/api/v1/staff/assisted")
        assert status == 200 and any(s["id"] == session and s["actions"] for s in view["sessions"])
        print(json.dumps({"result": "passed", "verification_method_enforced": True, "customer_authorization_required": True,
                          "confirmed_operations_only": True, "common_restriction_applies_to_customer": True, "customer_ends_session": True,
                          "audit_staff_customer_channel": {"authorizationRecords": traced, "decisions": decisions.split(",")}}))
    finally:
        if fixture.poll() is None:
            os.killpg(fixture.pid, signal.SIGTERM)
            fixture.wait(timeout=60)


if __name__ == "__main__":
    main()
