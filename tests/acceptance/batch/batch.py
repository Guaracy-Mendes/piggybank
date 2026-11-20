#!/usr/bin/env python3
"""WP-36 synthetic batch credit rail acceptance against the real local issuer, API and worker (the test is the network).

Proves: routing checksum and per-entry limit; common approval (action-bound native step-up) before any hold; cancel
before the window closes releases the hold; operator window close seals queued entries into one file; acknowledgement
rejections release holds; settlement posts one BATCH_CREDIT per entry even when repeated; a return inside the window posts
one linked BATCH_RETURN; a return after the window is flagged for review with no posting; signed messages replay their
stored response and bad signatures are refused. Ledger invariants hold.
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
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tests/mobile"))
import native_api as n  # noqa: E402

_sec = importlib.util.spec_from_file_location("security_acceptance", ROOT / "tests/acceptance/security/security.py")
security = importlib.util.module_from_spec(_sec)
_sec.loader.exec_module(security)
KEY = b"synthetic-batch-network-key-local-only"
CLEARING = "4f1a780b-3e31-48ef-a294-00000000ba70"


def psql(database, statement):
    return subprocess.check_output(["podman", "exec", "piggybank-postgres-1", "psql", "-U", "dev_admin", "-d", database, "-At", "-v", "ON_ERROR_STOP=1", "-c", statement], text=True).strip()


def network(document, key=KEY):
    body = json.dumps(document, separators=(",", ":")).encode()
    request = n.urllib.request.Request(n.API + "/channels/batch/v1/messages", data=body, method="POST",
                                       headers={"Content-Type": "application/json", "Piggybank-Batch-Signature": hmac.new(key, body, hashlib.sha256).hexdigest()})
    try:
        with n.urllib.request.urlopen(request, timeout=15) as response:
            raw = response.read()
            assert hmac.compare_digest(response.headers["piggybank-batch-signature"], hmac.new(KEY, raw, hashlib.sha256).hexdigest())
            return response.status, json.loads(raw), raw
    except n.urllib.error.HTTPError as error:
        return error.code, None, b""


def approve(alice, entry):
    challenge = entry["challengeId"]
    begin = alice.api("POST", f"/api/v1/approvals/{challenge}/native-step-up", expect=200)[1]
    time.sleep(1.1)
    proof = n.native_login("alice", nonce=begin["nonce"], fresh=True)
    assert alice.api("POST", f"/api/v1/approvals/{challenge}/native-proof", {"idToken": proof["id_token"]})[0] == 204
    return alice.api("POST", f"/api/v1/batch/payments/{entry['id']}/confirmation", expect=202)[1]


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
        account = n.open_account(alice, "Batchalice")
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), account, "100000"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        held = lambda: int(psql(database, f"SELECT held FROM core.ledger_accounts WHERE id='{account}'"))  # noqa: E731
        booked = lambda: int(psql(database, f"SELECT booked FROM core.ledger_accounts WHERE id='{account}'"))  # noqa: E731
        result = {}
        assert alice.api("POST", "/api/v1/batch/recipients", {"label": "Landlord", "routingNumber": "021000022", "accountNumber": "123456789"}, key=str(uuid.uuid4()))[0] == 409
        recipient = alice.api("POST", "/api/v1/batch/recipients", {"label": "Landlord", "routingNumber": "021000021", "accountNumber": "123456789"}, key=str(uuid.uuid4()), expect=201)[1]
        assert recipient["accountLast4"] == "6789" and "accountNumber" not in recipient
        body = lambda amount: {"accountId": account, "recipientId": recipient["id"], "amountMinor": amount}  # noqa: E731
        assert alice.api("POST", "/api/v1/batch/payments", body(2600000), key=str(uuid.uuid4()))[0] == 409, "per-entry limit"
        first = alice.api("POST", "/api/v1/batch/payments", body(30000), key=str(uuid.uuid4()), expect=201)[1]
        assert first["state"] == "DRAFT" and first["approvalUrl"].endswith(first["challengeId"]) and held() == 0
        assert alice.api("POST", f"/api/v1/batch/payments/{first['id']}/confirmation")[0] == 409, "confirmation without step-up"
        queued = approve(alice, first)
        assert queued["state"] == "QUEUED" and queued["settlementDate"] and held() == 30000
        result["common_approval_before_hold"] = True
        cancelled = alice.api("POST", "/api/v1/batch/payments", body(10000), key=str(uuid.uuid4()), expect=201)[1]
        approve(alice, cancelled)
        assert held() == 40000
        assert alice.api("POST", f"/api/v1/batch/payments/{cancelled['id']}/cancellation", expect=200)[1]["state"] == "CANCELLED" and held() == 30000
        third = alice.api("POST", "/api/v1/batch/payments", body(5000), key=str(uuid.uuid4()), expect=201)[1]
        approve(alice, third)
        fourth = alice.api("POST", "/api/v1/batch/payments", body(7000), key=str(uuid.uuid4()), expect=201)[1]
        approve(alice, fourth)
        assert held() == 42000
        result["cancel_before_cutoff_releases_hold"] = True
        # Operator closes the window: one file with the three queued entries.
        ops, csrf = security.workforce_login("reviewer")
        status, closed = security.staff(ops, csrf, "POST", "/api/v1/staff/batch/window-closure")
        assert status == 200 and closed["file"]["entryCount"] == 3, (status, closed)
        file_id = closed["file"]["fileId"]
        assert {e["entryId"] for e in closed["file"]["entries"]} == {first["id"], third["id"], fourth["id"]}
        assert alice.api("POST", f"/api/v1/batch/payments/{first['id']}/cancellation")[0] == 409, "no cancellation after submission"
        assert security.staff(ops, csrf, "POST", "/api/v1/staff/batch/window-closure")[1]["file"] is None, "empty window seals nothing"
        # Network acknowledgement with a rejection; duplicate replays the signed response.
        assert network({"messageId": "ACK-" + uuid.uuid4().hex[:16], "kind": "ACKNOWLEDGEMENT", "fileId": file_id}, key=b"wrong")[0] == 401
        ack = {"messageId": "ACK-" + uuid.uuid4().hex[:16], "kind": "ACKNOWLEDGEMENT", "fileId": file_id, "rejected": [{"entryId": third["id"], "reasonCode": "R03"}]}
        _, acked, raw1 = network(ack)
        _, _, raw2 = network(ack)
        assert acked["result"] == "ACKNOWLEDGED" and acked["rejected"] == 1 and raw1 == raw2 and held() == 37000
        assert network(dict(ack, rejected=[]))[0] == 409, "message id reuse with other content"
        result["rejection_releases_hold_and_replay"] = True
        # Settlement twice posts once per entry.
        for _ in range(2):
            _, settled, _ = network({"messageId": "SET-" + uuid.uuid4().hex[:16], "kind": "SETTLEMENT", "fileId": file_id})
            assert settled["result"] == "SETTLED"
        for e in (first, fourth):
            assert psql(database, f"SELECT count(*) FROM core.journal_batches WHERE operation_id='{e['id']}' AND posting_kind='BATCH_CREDIT'") == "1"
        assert held() == 0 and booked() == 63000
        result["settlement_posts_once"] = True
        # Return inside the window: one linked BATCH_RETURN; repeated return is idempotent.
        for _ in range(2):
            _, returned, _ = network({"messageId": "RET-" + uuid.uuid4().hex[:16], "kind": "RETURN", "entryId": first["id"], "reasonCode": "R02"})
            assert returned["result"] == "RETURNED"
        assert psql(database, f"SELECT count(*) FROM core.journal_batches r JOIN core.journal_batches c ON c.id=r.original_batch WHERE r.operation_id='{first['id']}' AND r.posting_kind='BATCH_RETURN' AND c.posting_kind='BATCH_CREDIT'") == "1"
        assert booked() == 93000
        assert alice.api("GET", f"/api/v1/batch/payments/{first['id']}", expect=200)[1]["state"] == "RETURNED"
        result["return_inside_window_posts_once_linked"] = True
        # Return after the window (clock fixture: back-date the settled entry's settlement date) is flagged, not posted.
        psql(database, f"UPDATE payments.batch_entries SET settlement_date=settlement_date-10 WHERE id='{fourth['id']}'")
        _, late, _ = network({"messageId": "RET-" + uuid.uuid4().hex[:16], "kind": "RETURN", "entryId": fourth["id"], "reasonCode": "R04"})
        assert late["result"] == "REVIEW_REQUIRED" and booked() == 93000
        assert network({"messageId": "RET-" + uuid.uuid4().hex[:16], "kind": "RETURN", "entryId": fourth["id"], "reasonCode": "R99"})[1]["reason"] == "UNKNOWN_RETURN_REASON"
        result["late_return_flagged_not_posted"] = True
        status, view = security.staff(ops, csrf, "GET", "/api/v1/staff/batch")
        assert status == 200 and any(f["id"] == file_id and f["state"] == "SETTLED" for f in view["files"])
        invariants = psql(database, "SELECT (SELECT count(*) FROM (SELECT batch_id FROM core.journal_entries GROUP BY batch_id HAVING sum(delta::numeric)<>0) u)||'|'||"
                                    "(SELECT count(*) FROM core.ledger_accounts a WHERE a.booked<>coalesce((SELECT sum(delta) FROM core.journal_entries e WHERE e.account_id=a.id),0))")
        assert invariants == "0|0" and int(psql(database, f"SELECT booked FROM core.ledger_accounts WHERE id='{CLEARING}'")) == 7000, invariants
        print(json.dumps({"result": "passed", **result, "invariants": "balanced"}))
    finally:
        if fixture.poll() is None:
            os.killpg(fixture.pid, signal.SIGTERM)
            fixture.wait(timeout=60)


if __name__ == "__main__":
    main()
