#!/usr/bin/env python3
"""WP-28 security acceptance against the real local issuer, API, worker and provider.

- FR-IAM-06 / AT-14: customer lost-device revocation and staff-assisted credential reset with independent
  approval. Issuer sessions, browser sessions and native refresh tokens are revoked; a reset forces a new password.
- SEC-02: the recovery executor's issuer secret rotates through a file, without a restart.
- SEC-05 / AT-19: service logs from the whole journey are inspected for seeded sensitive values.
The shared local issuer is restored at the end (required actions removed, default secret reinstated).
"""
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import urllib.parse
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tests/mobile"))
import importlib.util  # noqa: E402
import native_api as n  # noqa: E402

_spec = importlib.util.spec_from_file_location("issuer_fixtures", ROOT / "scripts/identity.py")
issuer = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(issuer)
admin_client, admin_token, admin = issuer.admin_client, issuer.admin_token, issuer.call

SECRET_DEFAULT = "synthetic-identity-admin-only"


def workforce_login(username):
    jar = n.cookies.CookieJar(policy=n.LoopbackCookiePolicy())
    opener = n.urllib.request.build_opener(n.urllib.request.HTTPCookieProcessor(jar))
    status, body, _ = n.bff_call(opener, "/auth/workforce/login")
    assert status == 200
    form = n.KeycloakForm()
    form.feed(body.decode())
    n.bff_call(opener, form.action, "POST", urllib.parse.urlencode({"username": username, "password": n.PASSWORD, "credentialId": ""}).encode(),
               {"Content-Type": "application/x-www-form-urlencoded"})
    csrf = next(c.value for c in jar if c.name == "pb_csrf_workforce")
    return opener, csrf


def staff(opener, csrf, method, path, body=None, key=None):
    headers = {"X-CSRF-Token": csrf, "Origin": "http://127.0.0.1:4301"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if key:
        headers["Idempotency-Key"] = key
    status, raw, _ = n.bff_call(opener, path, method, json.dumps(body).encode() if body is not None else (b"" if method == "POST" else None), headers)
    return status, (json.loads(raw) if raw[:1] in (b"{", b"[") else raw)


def user_id(token, username):
    return admin(token, "customers/users?exact=true&username=" + username)[0]["id"]


def refresh_ok(session):
    status, _ = n.http("POST", n.ISSUER + "/protocol/openid-connect/token", urllib.parse.urlencode({
        "grant_type": "refresh_token", "client_id": "bank-mobile", "refresh_token": session.tokens["refresh_token"]}).encode(),
        {"Content-Type": "application/x-www-form-urlencoded"})
    return status == 200


def main():
    logs = Path(tempfile.mkdtemp(prefix="security-logs-", dir=ROOT / ".local"))
    secret_file = logs / "identity-admin.secret"
    secret_file.write_text(SECRET_DEFAULT)
    secret_file.chmod(0o600)
    env = dict(os.environ, BANK_BROWSER_RATE_LIMIT="5000", BANK_FIXTURE_LOG_DIR=str(logs), BANK_IDENTITY_ADMIN_SECRET_FILE=str(secret_file))
    fixture = subprocess.Popen(["python3", str(ROOT / "tests/browser-customer/server.py")], cwd=ROOT, env=env, start_new_session=True, stdout=subprocess.DEVNULL)
    seeded = []
    try:
        for _ in range(240):
            try:
                if n.http("GET", n.API + "/health/ready")[0] == 200 and n.http("GET", "http://127.0.0.1:4301")[0] == 200:
                    break
            except OSError:
                pass
            time.sleep(0.5)
        # Seeded sensitive values that must never reach service logs.
        alice, bob = n.Session("alice"), n.Session("bob")
        n.open_account(alice, "Securityseed")
        n.open_account(bob, "Recipientseed")
        seeded += [n.PASSWORD, alice.tokens["access_token"], alice.tokens["refresh_token"], alice.tokens["id_token"], "Synthetic Securityseed",
                   "securityseed@example.invalid", "Synthetic native mobile identity evidence."]
        web, web_jar = n.web_login("alice")
        assert n.bff_call(web, "/auth/customer/me")[0] == 200
        # Customer self-service lost device: every issuer session and the browser session are revoked.
        status, revoked = alice.api("POST", "/api/v1/security/device-revocation", {"reason": "Phone was stolen"}, key=str(uuid.uuid4()))
        assert status == 200 and revoked["state"] == "EXECUTED", (status, revoked)
        assert not refresh_ok(alice), "native refresh token must be revoked"
        assert n.bff_call(web, "/auth/customer/me")[0] == 401
        alice = n.Session("alice")
        mine = alice.api("GET", "/api/v1/security/recoveries", expect=200)[1]
        assert mine[0]["state"] == "EXECUTED" and "risk" not in mine[0]
        messages = alice.api("GET", "/api/v1/notifications", expect=200)[1]
        for _ in range(30):
            if any(m["subject"] == "Your devices were signed out" for m in messages["messages"]):
                break
            time.sleep(1)
            messages = alice.api("GET", "/api/v1/notifications", expect=200)[1]
        assert any(m["subject"] == "Your devices were signed out" for m in messages["messages"])
        # Staff-assisted credential reset: support verifies ownership, an independent security approver decides.
        support, support_csrf = workforce_login("reviewer")
        approver, approver_csrf = workforce_login("approver")
        bob_person = n.claims(bob.tokens["access_token"])["sub"]
        status, persons = staff(support, support_csrf, "GET", "/api/v1/staff/security/recoveries")
        assert status == 200
        bob_link = subprocess.check_output(["podman", "exec", "piggybank-postgres-1", "psql", "-U", "dev_admin", "-d",
                                            json.loads((ROOT / ".local/customer-browser-fixture.json").read_text())["database"], "-At", "-c",
                                            f"SELECT person_id FROM controls.person_links WHERE subject='{uuid.UUID(bob_person)}'"], text=True).strip()
        status, pending = staff(support, support_csrf, "POST", "/api/v1/staff/security/recoveries",
                                {"personId": bob_link, "kind": "CREDENTIAL_RESET", "reason": "Customer verified on known contact", "verification": "VERIFIED_CONTACT"}, key=str(uuid.uuid4()))
        assert status == 200 and pending["state"] == "PENDING_APPROVAL", (status, pending)
        assert staff(support, support_csrf, "POST", f"/api/v1/staff/security/recoveries/{pending['id']}/decision", {"approve": True, "reason": "self"})[0] in (403, 409)
        # Rotate the executor's issuer secret through the file before the approval executes (no restart).
        client = admin(admin_token(), "customers/clients?clientId=bank-identity-admin")[0]["id"]
        rotated = admin(admin_token(), f"customers/clients/{client}/client-secret", "POST", {})["value"]
        secret_file.write_text(rotated)
        old = n.http("POST", n.ISSUER + "/protocol/openid-connect/token", urllib.parse.urlencode({
            "grant_type": "client_credentials", "client_id": "bank-identity-admin", "client_secret": SECRET_DEFAULT}).encode(), {"Content-Type": "application/x-www-form-urlencoded"})[0]
        assert old == 401, old
        seeded.append(rotated)
        status, executed = staff(approver, approver_csrf, "POST", f"/api/v1/staff/security/recoveries/{pending['id']}/decision",
                                 {"approve": True, "reason": "Ownership verification reviewed"})
        assert status == 200 and executed["state"] == "EXECUTED", (status, executed)
        assert executed["outcome"]["passwordUpdateRequired"] is True
        seeded.append("bob-required-action")
        assert "UPDATE_PASSWORD" in admin(admin_token(), "customers/users/" + user_id(admin_token(), "bob"))["requiredActions"]
        assert not refresh_ok(bob)
        # Recovery never discloses or sets a credential: the next login must set a new password itself.
        try:
            n.native_login("bob")
            raise AssertionError("login must stop at the required password update")
        except AssertionError as error:
            assert "custom-scheme redirect" in str(error) or "required password update" not in str(error)
        # Sensitive-data inspection over every captured service log.
        time.sleep(2)
        text = "\n".join(p.read_text(errors="replace") for p in logs.glob("*.log"))
        assert text.strip(), "service logs expected"
        leaks = [value[:12] + "…" for value in seeded if value != "bob-required-action" and value in text]
        assert not leaks, ("sensitive values in logs", leaks)
        print(json.dumps({"result": "passed", "self_device_revocation": True, "native_refresh_revoked": True, "browser_session_revoked": True,
                          "security_notice": True, "staff_reset_independent_approval": True, "password_update_required": True,
                          "secret_rotated_without_restart": True, "old_secret_rejected": True, "log_bytes_inspected": len(text), "seeded_values_absent": len(seeded) - 1}))
    finally:
        try:
            token = admin_token()
            uid = user_id(token, "bob")
            user = admin(token, "customers/users/" + uid)
            user["requiredActions"] = [a for a in user.get("requiredActions", []) if a != "UPDATE_PASSWORD"]
            admin(token, "customers/users/" + uid, "PUT", user)
            admin_client(admin_token())  # reinstate the default synthetic secret on the shared local issuer
        finally:
            os.killpg(fixture.pid, signal.SIGTERM)
            fixture.wait(timeout=60)
            for p in logs.glob("*"):
                p.unlink()
            logs.rmdir()


if __name__ == "__main__":
    main()
