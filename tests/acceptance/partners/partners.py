#!/usr/bin/env python3
"""WP-33 partner and delegation acceptance against the real local issuer, API, worker and provider.

- FR-IAM-05: registered partner clients (maker/checker), client-credentials tokens from a separate realm, scoped
  customer consent, per-client quota, expiry/revocation and HMAC-signed callbacks.
- FR-EXT-02: versioned partner contract: missing version refused, sunset version 410 with Sunset, deprecated
  version served with Deprecation and Sunset headers.
- AT-21: consent revoked while a partner-drafted, customer-approved payment is in flight: new partner calls fail
  at once, the payment still settles exactly once and stays traceable to the partner client and the customer.
- FR-ACC-08: an owner delegates PAY up to a limit; over-limit payments fail; revocation blocks new actions while
  the delegate's earlier intent keeps the delegate as its recorded actor.
"""
import hashlib
import hmac
import http.server
import json
import os
import signal
import subprocess
import sys
import threading
import time
import urllib.parse
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tests/mobile"))
sys.path.insert(0, str(ROOT / "tests/acceptance/security"))
import importlib.util  # noqa: E402
import native_api as n  # noqa: E402

_spec = importlib.util.spec_from_file_location("issuer_fixtures", ROOT / "scripts/identity.py")
issuer = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(issuer)
_sec = importlib.util.spec_from_file_location("security_acceptance", ROOT / "tests/acceptance/security/security.py")
security = importlib.util.module_from_spec(_sec)
_sec.loader.exec_module(security)

VERSION = "2026-11-01"
PARTNER_ISSUER = "http://127.0.0.1:18280/realms/partners"
HOOK_PORT = 4390
received = []


class Hooks(http.server.BaseHTTPRequestHandler):
    def do_POST(self):  # noqa: N802
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        received.append({"signature": self.headers.get("Piggybank-Signature", ""), "eventId": self.headers.get("Piggybank-Event-Id"), "body": body})
        self.send_response(204)
        self.end_headers()

    def log_message(self, *args):
        pass


def partner_token(client):
    status, body = n.http("POST", PARTNER_ISSUER + "/protocol/openid-connect/token", urllib.parse.urlencode({
        "grant_type": "client_credentials", "client_id": client, "client_secret": f"synthetic-{client}-only"}).encode(),
        {"Content-Type": "application/x-www-form-urlencoded"})
    assert status == 200, status
    return body["access_token"]


def pcall(token, method, path, body=None, consent=None, key=None, version=VERSION):
    headers = {"Authorization": "Bearer " + token}
    if version:
        headers["Piggybank-Api-Version"] = version
    if consent:
        headers["Consent-Id"] = consent
    if key:
        headers["Idempotency-Key"] = key
    request = n.urllib.request.Request(n.API + path, data=json.dumps(body).encode() if body is not None else None, method=method,
                                       headers={**headers, **({"Content-Type": "application/json"} if body is not None else {})})
    try:
        with n.urllib.request.urlopen(request, timeout=15) as response:
            raw = response.read()
            return response.status, (json.loads(raw) if raw else None), {k.lower(): v for k, v in response.headers.items()}
    except n.urllib.error.HTTPError as error:
        raw = error.read()
        try:
            parsed = json.loads(raw) if raw else None
        except ValueError:
            parsed = None
        return error.code, parsed, {k.lower(): v for k, v in error.headers.items()}


def psql(database, statement):
    return subprocess.check_output(["podman", "exec", "piggybank-postgres-1", "psql", "-U", "dev_admin", "-d", database, "-At", "-v", "ON_ERROR_STOP=1", "-c", statement], text=True).strip()


def wait(predicate, seconds, what):
    until = time.time() + seconds
    while time.time() < until:
        try:
            value = predicate()
        except OSError:
            value = None
        if value:
            return value
        time.sleep(0.5)
    raise AssertionError("timed out waiting for " + what)


def verify_signature(hook, secret):
    parts = dict(p.split("=", 1) for p in hook["signature"].split(","))
    expected = hmac.new(secret.encode(), parts["t"].encode() + b"." + hook["body"], hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected, parts["v1"]) and abs(time.time() - int(parts["t"])) < 300


def approve_and_confirm(alice, payment_id, quote_id):
    challenge = alice.api("POST", f"/api/v1/payments/{payment_id}/approval", expect=200)[1]["challengeId"]
    begin = alice.api("POST", f"/api/v1/approvals/{challenge}/native-step-up", expect=200)[1]
    time.sleep(1.1)
    proof = n.native_login("alice", nonce=begin["nonce"], fresh=True)
    assert alice.api("POST", f"/api/v1/approvals/{challenge}/native-proof", {"idToken": proof["id_token"]})[0] == 204
    assert alice.api("POST", f"/api/v1/payments/{payment_id}/confirmation", {"quoteId": quote_id}, expect=202)[1]["id"] == payment_id


def main():
    env = dict(os.environ, BANK_BROWSER_RATE_LIMIT="5000")
    fixture = subprocess.Popen(["python3", str(ROOT / "tests/browser-customer/server.py")], cwd=ROOT, env=env, start_new_session=True, stdout=subprocess.DEVNULL)
    hooks = http.server.ThreadingHTTPServer(("127.0.0.1", HOOK_PORT), Hooks)
    threading.Thread(target=hooks.serve_forever, daemon=True).start()
    result = {}
    try:
        wait(lambda: fixture.poll() is None and n.http("GET", n.API + "/health/ready")[0] == 200, 120, "fixture readiness")
        time.sleep(1)
        assert fixture.poll() is None, "fixture exited (ports already bound?)"
        database = json.loads((ROOT / ".local/customer-browser-fixture.json").read_text())["database"]
        issuer.partner_realm(issuer.admin_token())
        # ----- Registry: maker/checker -----
        sec, sec_csrf = security.workforce_login("security")
        status, proposed = security.staff(sec, sec_csrf, "POST", "/api/v1/staff/partners", {
            "clientId": "partner-budget", "displayName": "Synthetic Budget App", "allowedScopes": ["accounts:read", "payments:initiate"],
            "quotaPerMinute": 60, "callbackUrl": f"http://127.0.0.1:{HOOK_PORT}/hooks"}, key=str(uuid.uuid4()))
        assert status == 201 and proposed["client"]["state"] == "PENDING_APPROVAL" and len(proposed["callbackSecret"]) == 64, (status, proposed)
        secret, budget_id = proposed["callbackSecret"], proposed["client"]["id"]
        budget = partner_token("partner-budget")
        assert pcall(budget, "POST", "/partner/v1/consents", {"scopes": ["accounts:read"], "durationDays": 30}, key=str(uuid.uuid4()))[0] == 401, "pending client accepted"
        status, _ = security.staff(sec, sec_csrf, "POST", f"/api/v1/staff/partners/{budget_id}/decision", {"decision": "APPROVE", "reason": "self approval"})
        assert status in (403, 409), status
        appr, appr_csrf = security.workforce_login("approver")
        status, decided = security.staff(appr, appr_csrf, "POST", f"/api/v1/staff/partners/{budget_id}/decision", {"decision": "APPROVE", "reason": "synthetic contract reviewed"})
        assert status == 200 and decided["state"] == "ACTIVE", (status, decided)
        result["registry_maker_checker"] = True
        # ----- Versioning and unregistered clients -----
        other = partner_token("partner-other")
        assert pcall(other, "POST", "/partner/v1/consents", {"scopes": ["accounts:read"], "durationDays": 1}, key=str(uuid.uuid4()))[0] == 401
        assert pcall(budget, "GET", f"/partner/v1/consents/{uuid.uuid4()}", version=None)[0] == 400
        gone, _, headers = pcall(budget, "GET", f"/partner/v1/consents/{uuid.uuid4()}", version="2026-05-01")
        assert gone == 410 and "sunset" in headers, (gone, headers)
        result["sunset_version_410"] = True
        # ----- Customers, accounts and a recipient -----
        alice, bob = n.Session("alice"), n.Session("bob")
        source, target = n.open_account(alice, "Partnerpayer"), n.open_account(bob, "Partnerpayee")
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), source, "500000"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        reference = bob.api("POST", f"/api/v1/accounts/{target}/receiving-reference", expect=200)[1]["reference"]
        beneficiary = alice.api("POST", "/api/v1/beneficiaries", {"label": "Bob partner", "receivingReference": reference}, key=str(uuid.uuid4()), expect=201)[1]["id"]
        # ----- Consent request and customer authorization -----
        status, pending, _ = pcall(budget, "POST", "/partner/v1/consents", {"scopes": ["accounts:read", "payments:initiate"], "durationDays": 30}, key=str(uuid.uuid4()))
        assert status == 201 and pending["state"] == "AWAITING_AUTHORIZATION" and pending["authorizationUrl"].endswith("/#/connected/" + pending["consentId"]), pending
        consent = pending["consentId"]
        assert pcall(budget, "GET", "/partner/v1/accounts", consent=consent)[0] == 403, "unauthorized consent served data"
        assert alice.api("POST", f"/api/v1/consents/{consent}/authorization", {"approve": True, "accountIds": [target]})[0] in (400, 409)
        assert alice.api("POST", f"/api/v1/consents/{consent}/authorization", {"approve": True, "accountIds": [source]}, expect=200)[1]["state"] == "ACTIVE"
        assert bob.api("GET", f"/api/v1/consents/{consent}")[0] == 404, "another customer could see a bound consent"
        status, accounts, _ = pcall(budget, "GET", "/partner/v1/accounts", consent=consent)
        assert status == 200 and [a["accountId"] for a in accounts["accounts"]] == [source], accounts
        assert pcall(budget, "GET", f"/partner/v1/accounts/{target}/balance", consent=consent)[0] == 404
        day = time.strftime("%Y-%m-%d", time.gmtime())
        tomorrow = time.strftime("%Y-%m-%d", time.gmtime(time.time() + 86400))
        status, page, _ = pcall(budget, "GET", f"/partner/v1/accounts/{source}/transactions?from={day}&to={tomorrow}&limit=10", consent=consent)
        assert status == 200 and len(page["items"]) >= 1, (status, page)
        assert pcall(other, "GET", "/partner/v1/accounts", consent=consent)[0] == 401
        status, _, headers = pcall(budget, "GET", f"/partner/v1/consents/{consent}", version="2026-08-01")
        assert status == 200 and headers.get("deprecation") == "true" and "sunset" in headers, headers
        result["deprecated_version_headers"] = True
        hook = wait(lambda: next((h for h in received if json.loads(h["body"])["eventType"] == "consent.authorized"), None), 30, "consent.authorized callback")
        assert verify_signature(hook, secret) and json.loads(hook["body"])["consent"]["consentId"] == consent
        forged = dict(hook, body=hook["body"] + b" ")
        assert not verify_signature(forged, secret)
        result["signed_callback_verified"] = True
        # ----- Partner-drafted payment, approved by the customer, in flight at revocation (AT-21) -----
        status, recipients, _ = pcall(budget, "GET", "/partner/v1/beneficiaries", consent=consent)
        assert status == 200 and [r["id"] for r in recipients["beneficiaries"]] == [beneficiary] and "receivingReference" not in json.dumps(recipients)
        quote_body = {"payerAccountId": source, "beneficiaryId": beneficiary, "amount": {"minorUnits": "1500", "currency": {"code": "TST", "scale": 2}}, "requestedDate": day}
        status, quote, _ = pcall(budget, "POST", "/partner/v1/payment-quotes", quote_body, consent=consent)
        assert status == 201, (status, quote)
        key = str(uuid.uuid4())
        status, draft, _ = pcall(budget, "POST", "/partner/v1/payments", {"quoteId": quote["id"]}, consent=consent, key=key)
        assert status == 201 and draft["state"] == "DRAFT" and draft["customerApprovalRequired"] and "approvalUrl" not in draft, draft
        assert pcall(budget, "POST", "/partner/v1/payments", {"quoteId": quote["id"]}, consent=consent, key=key)[1]["id"] == draft["id"], "same key must replay"
        payment = draft["id"]
        listed = [p for p in alice.api("GET", "/api/v1/payments", expect=200)[1] if p["id"] == payment]
        assert listed and listed[0]["state"] == "DRAFT", "customer must see the partner draft to approve it"
        approve_and_confirm(alice, payment, quote["id"])
        revoked = alice.api("POST", f"/api/v1/consents/{consent}/revocation", {"reason": "stopped using the app"}, expect=200)[1]
        assert revoked["state"] == "REVOKED" and revoked["revokedBy"] == "CUSTOMER"
        for path in ["/partner/v1/accounts", f"/partner/v1/payments/{payment}", f"/partner/v1/accounts/{source}/balance"]:
            assert pcall(budget, "GET", path, consent=consent)[0] == 403, path
        assert pcall(budget, "POST", "/partner/v1/payment-quotes", quote_body, consent=consent)[0] == 403
        settled = wait(lambda: alice.api("GET", f"/api/v1/payments/{payment}", expect=200)[1]["state"] == "SETTLED", 120, "payment settlement after revocation")
        evidence = psql(database, f"SELECT client_id||'|'||(actor_context->>'trust')||'|'||issuer||'|'||(SELECT count(*) FROM core.journal_batches b WHERE b.id=i.batch_id) FROM payments.intents i WHERE id='{payment}'")
        client, trust, payer_issuer, batches = evidence.split("|")
        assert (client, trust, batches) == ("partner-budget", "PARTNER", "1") and payer_issuer.endswith("/realms/customers"), evidence
        wait(lambda: any(json.loads(h["body"])["eventType"] == "consent.revoked" for h in received), 30, "consent.revoked callback")
        result["AT21_revocation_in_flight"] = {"newCallsRefused": True, "settledOnce": bool(settled), "actorEvidence": {"clientId": client, "trust": trust}}
        # ----- Quota and cross-partner isolation -----
        status, other_proposed = security.staff(sec, sec_csrf, "POST", "/api/v1/staff/partners", {
            "clientId": "partner-other", "displayName": "Synthetic Other App", "allowedScopes": ["accounts:read"], "quotaPerMinute": 3}, key=str(uuid.uuid4()))
        assert status == 201 and other_proposed["callbackSecret"] is None
        security.staff(appr, appr_csrf, "POST", f"/api/v1/staff/partners/{other_proposed['client']['id']}/decision", {"decision": "APPROVE", "reason": "synthetic"})
        statuses = [pcall(other, "GET", f"/partner/v1/consents/{consent}")[0] for _ in range(4)]
        assert statuses[:3] == [404, 404, 404] and statuses[3] == 429, statuses
        assert pcall(other, "POST", "/partner/v1/consents", {"scopes": ["payments:initiate"], "durationDays": 1}, key=str(uuid.uuid4()))[0] == 429
        result["quota_and_client_isolation"] = True
        # ----- Delegation with bounded authority (FR-ACC-08) -----
        code = bob.api("POST", "/api/v1/delegations/reference", expect=200)[1]["reference"]
        granted = alice.api("POST", "/api/v1/delegations", {"accountId": source, "reference": code, "authority": "PAY", "paymentLimitMinor": 2000, "durationDays": 7},
                            key=str(uuid.uuid4()), expect=201)[1]
        assert bob.api("GET", f"/api/v1/accounts/{source}/balance", expect=200)[1]["accountId"] == source
        bob_recipient = bob.api("POST", "/api/v1/beneficiaries", {"label": "Own account", "receivingReference": reference}, key=str(uuid.uuid4()), expect=201)[1]["id"]
        small = dict(quote_body, beneficiaryId=bob_recipient, amount={"minorUnits": "1000", "currency": {"code": "TST", "scale": 2}})
        delegate_quote = bob.api("POST", "/api/v1/payment-quotes", small, expect=201)[1]
        delegate_payment = bob.api("POST", "/api/v1/payments", {"quoteId": delegate_quote["id"]}, key=str(uuid.uuid4()), expect=201)[1]["id"]
        assert bob.api("POST", "/api/v1/payment-quotes", dict(small, amount={"minorUnits": "2001", "currency": {"code": "TST", "scale": 2}}))[0] == 409
        assert bob.api("POST", f"/api/v1/accounts/{source}/closure")[0] in (403, 404, 405, 409), "delegate must not service the account"
        alice.api("POST", f"/api/v1/delegations/{granted['id']}/revocation", {"reason": "trip finished"}, expect=200)
        assert bob.api("GET", f"/api/v1/accounts/{source}/balance")[0] == 404
        assert bob.api("POST", "/api/v1/payment-quotes", small)[0] in (404, 409)
        actor_subject = psql(database, f"SELECT actor_context->>'subject' FROM payments.intents WHERE id='{delegate_payment}'")
        assert actor_subject == n.claims(bob.tokens["access_token"])["sub"], "delegate actor evidence must be preserved"
        result["delegation_bounded_and_revocable"] = True
        # ----- Suspension stops the client entirely -----
        security.staff(sec, sec_csrf, "POST", f"/api/v1/staff/partners/{budget_id}/decision", {"decision": "SUSPEND", "reason": "security review"})
        assert pcall(partner_token("partner-budget"), "GET", f"/partner/v1/consents/{consent}")[0] == 401
        status, view = security.staff(appr, appr_csrf, "GET", "/api/v1/staff/partners")
        assert status == 200 and any(c["eventType"] == "consent.revoked" and c["state"] == "DELIVERED" for c in view["callbacks"])
        result["suspension_blocks_client"] = True
        print(json.dumps({"result": "passed", **result}))
    finally:
        hooks.shutdown()
        if fixture.poll() is None:
            os.killpg(fixture.pid, signal.SIGTERM)
            fixture.wait(timeout=60)


if __name__ == "__main__":
    main()
