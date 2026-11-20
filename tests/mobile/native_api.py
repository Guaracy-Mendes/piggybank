#!/usr/bin/env python3
"""WP-27 native-client contract against the real local issuer, API, worker and provider.

Acts exactly like the mobile app: PKCE authorization code for the public `bank-mobile` client with the
`piggybank://auth/callback` redirect, bearer API calls, rotating refresh tokens, native action-bound
step-up and persisted-operation recovery. Credentials and tokens are never printed.
"""
import base64
import hashlib
import http.cookiejar as cookies
import json
import os
import secrets
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
API = "http://127.0.0.1:4180"
ISSUER = "http://127.0.0.1:18280/realms/customers"
REDIRECT = "piggybank://auth/callback"
PASSWORD = "Synthetic-Local-123!"
sys.path.insert(0, str(ROOT / "tests/integration"))
from identity import Form as KeycloakForm, LoopbackCookiePolicy, call as bff_call  # noqa: E402


class Form(HTMLParser):
    def __init__(self):
        super().__init__()
        self.action = None

    def handle_starttag(self, tag, attrs):
        values = dict(attrs)
        if tag == "form" and values.get("id") == "kc-form-login":
            self.action = values["action"]


class Capture(urllib.request.HTTPRedirectHandler):
    """Stops at the custom-scheme redirect, as the OS hands it to the app."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if newurl.startswith(REDIRECT):
            raise Redirected(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


class Redirected(Exception):
    def __init__(self, url):
        super().__init__("redirected")
        self.url = url


def b64(data):
    return base64.urlsafe_b64encode(data).decode().rstrip("=")


def claims(jwt):
    return json.loads(base64.urlsafe_b64decode(jwt.split(".")[1] + "==="))


def http(method, url, body=None, headers=None):
    data = None
    headers = dict(headers or {})
    if isinstance(body, (dict, list)):
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    elif isinstance(body, bytes):
        data = body
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        response = urllib.request.urlopen(req, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read()
        return response.status, (json.loads(raw) if raw and raw[:1] in b"{[" else raw)


def native_login(username, nonce=None, fresh=False):
    """Authorization code + PKCE (S256) for the public native client."""
    verifier = b64(secrets.token_bytes(48))
    state = b64(secrets.token_bytes(16))
    nonce = nonce or b64(secrets.token_bytes(16))
    query = {
        "client_id": "bank-mobile",
        "redirect_uri": REDIRECT,
        "response_type": "code",
        "scope": "openid",
        "state": state,
        "nonce": nonce,
        "code_challenge": b64(hashlib.sha256(verifier.encode()).digest()),
        "code_challenge_method": "S256",
    }
    if fresh:
        query.update({"prompt": "login", "max_age": "0"})
    jar = cookies.CookieJar(policy=LoopbackCookiePolicy())
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar), Capture())
    with opener.open(ISSUER + "/protocol/openid-connect/auth?" + urllib.parse.urlencode(query), timeout=15) as page:
        form = Form()
        form.feed(page.read().decode())
    assert form.action, "issuer login form expected"
    location = None
    try:
        opener.open(form.action, urllib.parse.urlencode({"username": username, "password": PASSWORD, "credentialId": ""}).encode(), timeout=15)
    except Redirected as r:
        location = r.url
    except urllib.error.HTTPError as error:
        # urllib refuses non-HTTP schemes; the OS would hand this redirect to the app.
        if error.code in (302, 303) and (error.headers.get("Location") or "").startswith(REDIRECT):
            location = error.headers["Location"]
        error.close()
    assert location, "expected custom-scheme redirect"
    params = dict(urllib.parse.parse_qsl(urllib.parse.urlsplit(location).query))
    assert params["state"] == state and params.get("iss", ISSUER) == ISSUER
    status, tokens = http("POST", ISSUER + "/protocol/openid-connect/token", urllib.parse.urlencode({
        "grant_type": "authorization_code", "client_id": "bank-mobile", "code": params["code"],
        "code_verifier": verifier, "redirect_uri": REDIRECT}).encode(), {"Content-Type": "application/x-www-form-urlencoded"})
    assert status == 200, ("token exchange", status)
    assert claims(tokens["id_token"])["nonce"] == nonce
    return tokens


class Session:
    """Mirrors the app: short access tokens, single-use rotating refresh tokens."""

    def __init__(self, username):
        self.username = username
        self.tokens = native_login(username)

    def refresh(self):
        status, tokens = http("POST", ISSUER + "/protocol/openid-connect/token", urllib.parse.urlencode({
            "grant_type": "refresh_token", "client_id": "bank-mobile", "refresh_token": self.tokens["refresh_token"]}).encode(),
            {"Content-Type": "application/x-www-form-urlencoded"})
        assert status == 200, ("refresh", status)
        self.tokens = tokens

    def api(self, method, path, body=None, key=None, expect=None):
        if claims(self.tokens["access_token"])["exp"] - time.time() < 5:
            self.refresh()
        headers = {"Authorization": "Bearer " + self.tokens["access_token"]}
        if key:
            headers["Idempotency-Key"] = key
        status, payload = http(method, API + path, body, headers)
        if expect is not None:
            assert status == expect, (method, path, status, payload if status >= 400 else "")
        return status, payload


def open_account(s, label):
    products = s.api("GET", "/api/v1/products", expect=200)[1]
    product = products[0]["id"]
    app = s.api("POST", "/api/v1/applications", {"productVersionId": product, "marketing": False}, key=str(uuid.uuid4()), expect=201)[1]
    path = "/api/v1/applications/" + app["id"]
    s.api("PUT", path + "/profile", {"version": app["version"], "profile": {"fullName": "Synthetic " + label, "dateOfBirth": "1990-01-01",
          "email": label.lower() + "@example.invalid", "residency": "SYNTHETIC", "consentVersion": "synthetic-consent-v1", "marketing": False}}, expect=200)
    s.api("POST", path + "/evidence", {"content": "Synthetic native mobile identity evidence."}, expect=200)
    terms = s.api("GET", f"/api/v1/products/{product}/terms", expect=200)[1]
    s.api("POST", path + "/terms", {"termsId": terms["id"]}, expect=200)
    s.api("POST", path + "/submit", key=str(uuid.uuid4()), expect=202)
    for _ in range(120):
        app = s.api("GET", path, expect=200)[1]
        if app["state"] == "ACTIVE":
            return app["accountId"]
        time.sleep(0.5)
    raise AssertionError("account not active: " + app["state"])


def web_login(username):
    """Browser BFF session for the same person (customer-web client, cookie session)."""
    jar = cookies.CookieJar(policy=LoopbackCookiePolicy())
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    status, body, _ = bff_call(opener, "/auth/customer/login")
    assert status == 200, status
    form = KeycloakForm()
    form.feed(body.decode())
    assert form.action
    bff_call(opener, form.action, "POST", urllib.parse.urlencode({"username": username, "password": PASSWORD, "credentialId": ""}).encode(),
             {"Content-Type": "application/x-www-form-urlencoded"})
    assert any(c.name == "pb_customer" for c in jar), "BFF session cookie expected"
    return opener, jar


def main():
    env = dict(os.environ, BANK_BROWSER_RATE_LIMIT="5000")
    fixture = subprocess.Popen(["python3", str(ROOT / "tests/browser-customer/server.py")], cwd=ROOT, env=env, start_new_session=True, stdout=subprocess.DEVNULL)
    try:
        for _ in range(240):
            try:
                if http("GET", API + "/health/ready")[0] == 200 and http("GET", "http://127.0.0.1:4300")[0] == 200:
                    break
            except OSError:
                pass
            time.sleep(0.5)
        else:
            raise AssertionError("fixture readiness timeout")
        database = json.loads((ROOT / ".local/customer-browser-fixture.json").read_text())["database"]
        alice, bob = Session("alice"), Session("bob")
        assert claims(alice.tokens["access_token"])["azp"] == "bank-mobile"
        assert claims(alice.tokens["access_token"])["exp"] - claims(alice.tokens["access_token"])["iat"] <= 60
        source, target = open_account(alice, "Mobile"), open_account(bob, "Recipient")
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), source, "100000"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        # AT-01: own account matches across channels; another customer's account ID is denied without leakage.
        accounts = alice.api("GET", "/api/v1/accounts", expect=200)[1]
        assert [a["id"] for a in accounts] == [source]
        assert alice.api("GET", f"/api/v1/accounts/{source}/balance", expect=200)[1]["available"]["minorUnits"] == "100000"
        denied, body = bob.api("GET", f"/api/v1/accounts/{source}/balance")
        assert denied in (403, 404) and "100000" not in json.dumps(body, default=str)
        # Payment: persisted intent, interrupted response recovers the same operation (no second payment).
        reference = bob.api("POST", f"/api/v1/accounts/{target}/receiving-reference", expect=200)[1]["reference"]
        beneficiary = alice.api("POST", "/api/v1/beneficiaries", {"label": "Bob mobile", "receivingReference": reference}, key=str(uuid.uuid4()), expect=201)[1]
        quote = alice.api("POST", "/api/v1/payment-quotes", {"payerAccountId": source, "beneficiaryId": beneficiary["id"],
                          "amount": {"minorUnits": "2500", "currency": {"code": "TST", "scale": 2}}, "requestedDate": time.strftime("%Y-%m-%d", time.gmtime())}, expect=201)[1]
        key = str(uuid.uuid4())
        first = alice.api("POST", "/api/v1/payments", {"quoteId": quote["id"]}, key=key, expect=201)[1]
        retried = alice.api("POST", "/api/v1/payments", {"quoteId": quote["id"]}, key=key)[1]
        assert retried["id"] == first["id"]
        assert len([p for p in alice.api("GET", "/api/v1/payments", expect=200)[1] if p["quoteId"] == quote["id"]]) == 1
        # Native action-bound step-up.
        payment = alice.api("POST", f"/api/v1/payments/{first['id']}/approval", expect=200)[1]
        challenge = payment["challengeId"]
        stale = alice.tokens["id_token"]
        begin = alice.api("POST", f"/api/v1/approvals/{challenge}/native-step-up", expect=200)[1]
        assert begin["prompt"] == "login" and begin["maxAge"] == 0
        assert alice.api("POST", f"/api/v1/approvals/{challenge}/native-proof", {"idToken": stale})[0] == 401
        assert bob.api("POST", f"/api/v1/approvals/{challenge}/native-step-up")[0] == 404
        wrong = native_login("alice", nonce="not-the-challenge-nonce", fresh=True)
        assert alice.api("POST", f"/api/v1/approvals/{challenge}/native-proof", {"idToken": wrong["id_token"]})[0] == 401
        time.sleep(1.1)
        fresh = native_login("alice", nonce=begin["nonce"], fresh=True)
        assert alice.api("POST", f"/api/v1/approvals/{challenge}/native-proof", {"idToken": fresh["id_token"]})[0] == 204
        assert alice.api("POST", f"/api/v1/approvals/{challenge}/native-proof", {"idToken": fresh["id_token"]})[0] == 404
        assert alice.api("GET", "/api/v1/payments/" + first["id"], expect=200)[1]["approvalReady"] is True
        confirmed = alice.api("POST", f"/api/v1/payments/{first['id']}/confirmation", {"quoteId": quote["id"]}, expect=202)[1]
        assert confirmed["id"] == first["id"]
        for _ in range(60):
            state = alice.api("GET", "/api/v1/payments/" + first["id"], expect=200)[1]["state"]
            if state == "SETTLED":
                break
            time.sleep(0.5)
        assert state == "SETTLED", state
        # The same person on the browser channel sees the same account and payment identities.
        web, web_jar = web_login("alice")
        status, raw, _ = bff_call(web, "/api/v1/accounts")
        assert status == 200 and [a["id"] for a in json.loads(raw)] == [source]
        status, raw, _ = bff_call(web, "/api/v1/payments/" + first["id"])
        assert status == 200 and json.loads(raw)["id"] == first["id"] and json.loads(raw)["state"] == "SETTLED"
        # A browser session (customer-web client) cannot use the native-only reauthentication path.
        second_quote = alice.api("POST", "/api/v1/payment-quotes", {"payerAccountId": source, "beneficiaryId": beneficiary["id"],
                                 "amount": {"minorUnits": "100", "currency": {"code": "TST", "scale": 2}}, "requestedDate": time.strftime("%Y-%m-%d", time.gmtime())}, expect=201)[1]
        second = alice.api("POST", "/api/v1/payments", {"quoteId": second_quote["id"]}, key=str(uuid.uuid4()), expect=201)[1]
        web_challenge = alice.api("POST", f"/api/v1/payments/{second['id']}/approval", expect=200)[1]["challengeId"]
        csrf = next(c.value for c in web_jar if c.name == "pb_csrf_customer")
        status, _, _ = bff_call(web, f"/api/v1/approvals/{web_challenge}/native-step-up", "POST", b"", {"X-CSRF-Token": csrf})
        assert status == 403, status
        assert alice.api("GET", f"/api/v1/accounts/{source}/balance", expect=200)[1]["booked"]["minorUnits"] == "97500"
        # Documents and messages are reachable with the native bearer.
        assert "accounts" in alice.api("GET", "/api/v1/documents", expect=200)[1]
        assert "purposes" in alice.api("GET", "/api/v1/notifications", expect=200)[1]
        # Refresh tokens rotate once; sign-out revokes the device's refresh token.
        old = alice.tokens["refresh_token"]
        alice.refresh()
        status, _ = http("POST", ISSUER + "/protocol/openid-connect/token", urllib.parse.urlencode({
            "grant_type": "refresh_token", "client_id": "bank-mobile", "refresh_token": old}).encode(), {"Content-Type": "application/x-www-form-urlencoded"})
        assert status == 400
        # Reuse of a rotated refresh token is treated as theft: the issuer revokes the newer token too.
        status, _ = http("POST", ISSUER + "/protocol/openid-connect/token", urllib.parse.urlencode({
            "grant_type": "refresh_token", "client_id": "bank-mobile", "refresh_token": alice.tokens["refresh_token"]}).encode(), {"Content-Type": "application/x-www-form-urlencoded"})
        assert status == 400
        alice = Session("alice")
        status, _ = http("POST", ISSUER + "/protocol/openid-connect/revoke", urllib.parse.urlencode({
            "client_id": "bank-mobile", "token": alice.tokens["refresh_token"], "token_type_hint": "refresh_token"}).encode(), {"Content-Type": "application/x-www-form-urlencoded"})
        assert status == 200
        status, _ = http("POST", ISSUER + "/protocol/openid-connect/token", urllib.parse.urlencode({
            "grant_type": "refresh_token", "client_id": "bank-mobile", "refresh_token": alice.tokens["refresh_token"]}).encode(), {"Content-Type": "application/x-www-form-urlencoded"})
        assert status == 400
        print(json.dumps({"result": "passed", "native_pkce": True, "cross_channel_identities": True, "cross_customer_denied": True,
                          "interrupted_payment_recovered": True, "native_step_up": True, "settled": True, "refresh_rotation": True, "refresh_reuse_revokes_chain": True, "device_sign_out": True}))
    finally:
        os.killpg(fixture.pid, signal.SIGTERM)
        fixture.wait(timeout=60)


if __name__ == "__main__":
    main()
