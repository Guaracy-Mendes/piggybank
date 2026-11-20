#!/usr/bin/env python3
"""WP-35 ATM channel acceptance (FR-CH-04, AT-22/AT-08) against the real local API and worker, acting as the network.

Proves: signed channel authentication; unsupported operations declined with no effect; PIN assertion required;
authorization holds funds; duplicate messages replay the stored response; message-id reuse conflicts; duplicate
completions post once; late reversal after completion is flagged, never auto-posted; reversal before completion
releases the hold; per-withdrawal and daily limits; network timeout expiry releases the hold; a restricted account and a
blocked card are declined; settlement reconciliation lists every difference. Ledger invariants hold throughout.
"""
import hashlib
import hmac
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

KEY = b"synthetic-atm-network-key-local-only"
SETTLEMENT = "4f1a780b-3e31-48ef-a294-00000000a7e0"


def psql(database, statement):
    return subprocess.check_output(["podman", "exec", "piggybank-postgres-1", "psql", "-U", "dev_admin", "-d", database, "-At", "-v", "ON_ERROR_STOP=1", "-c", statement], text=True).strip()


def network(path, document, key=KEY):
    body = json.dumps(document, separators=(",", ":")).encode()
    signature = hmac.new(key, body, hashlib.sha256).hexdigest()
    request = n.urllib.request.Request(n.API + path, data=body, method="POST", headers={"Content-Type": "application/json", "Piggybank-Atm-Signature": signature})
    try:
        with n.urllib.request.urlopen(request, timeout=15) as response:
            raw = response.read()
            sig = response.headers.get("piggybank-atm-signature")
            if path.endswith("/messages"):
                assert sig and hmac.compare_digest(sig, hmac.new(KEY, raw, hashlib.sha256).hexdigest()), "unsigned or wrongly signed response"
            return response.status, json.loads(raw), raw
    except n.urllib.error.HTTPError as error:
        return error.code, None, b""


def msg(token, kind, **fields):
    return {"messageId": "SIM-" + uuid.uuid4().hex[:20], "kind": kind, "terminalId": "SIM-ATM-001", "cardToken": token, "pinVerified": True, **fields}


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
        account = n.open_account(alice, "Atmalice")
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), account, "100000"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        card = alice.api("POST", "/api/v1/atm/cards", {"accountId": account}, key=str(uuid.uuid4()), expect=201)[1]
        token = card["token"]
        held = lambda: int(psql(database, f"SELECT held FROM core.ledger_accounts WHERE id='{account}'"))  # noqa: E731
        result = {}
        # Channel authentication and the supplement.
        assert network("/channels/atm/v1/messages", msg(token, "INQUIRY"), key=b"wrong-key")[0] == 401
        assert network("/channels/atm/v1/messages", msg(token, "INQUIRY", pinVerified=False))[1]["reason"] == "CHANNEL_AUTHENTICATION_FAILED"
        for op in ["DEPOSIT", "TRANSFER", "PIN_CHANGE", "MINI_STATEMENT"]:
            assert network("/channels/atm/v1/messages", msg(token, "AUTHORIZATION", operation=op, amountMinor=1000))[1]["reason"] == "OPERATION_NOT_SUPPORTED", op
        assert held() == 0
        status, inquiry, _ = network("/channels/atm/v1/messages", msg(token, "INQUIRY"))
        assert inquiry["result"] == "APPROVED" and inquiry["availableMinor"] == "100000", inquiry
        result["unsupported_operations_declined_without_effect"] = True
        # Authorization, duplicate replay and message-id reuse.
        auth_message = msg(token, "AUTHORIZATION", operation="CASH_WITHDRAWAL", amountMinor=20000)
        _, first, first_raw = network("/channels/atm/v1/messages", auth_message)
        assert first["result"] == "APPROVED" and held() == 20000, first
        _, again, again_raw = network("/channels/atm/v1/messages", auth_message)
        assert again_raw == first_raw and held() == 20000, "duplicate must replay the stored response with no second hold"
        assert network("/channels/atm/v1/messages", dict(auth_message, amountMinor=30000))[0] == 409
        auth = first["authorizationId"]
        # Two completions (different message ids) post once.
        _, done1, _ = network("/channels/atm/v1/messages", msg(token, "COMPLETION", authorizationId=auth))
        _, done2, _ = network("/channels/atm/v1/messages", msg(token, "COMPLETION", authorizationId=auth))
        assert done1["result"] == done2["result"] == "COMPLETED" and done1["batchId"] == done2["batchId"]
        assert psql(database, f"SELECT count(*) FROM core.journal_batches WHERE operation_id='{auth}' AND posting_kind='ATM_WITHDRAWAL'") == "1" and held() == 0
        booked = int(psql(database, f"SELECT booked FROM core.ledger_accounts WHERE id='{account}'"))
        assert booked == 80000, booked
        result["duplicate_completion_posts_once"] = True
        # Late reversal after completion is flagged for review, never posted.
        _, late, _ = network("/channels/atm/v1/messages", msg(token, "REVERSAL", authorizationId=auth))
        assert late["result"] == "REVIEW_REQUIRED" and int(psql(database, f"SELECT booked FROM core.ledger_accounts WHERE id='{account}'")) == 80000
        # Reversal before completion releases the hold; a later completion is refused.
        _, second, _ = network("/channels/atm/v1/messages", msg(token, "AUTHORIZATION", operation="CASH_WITHDRAWAL", amountMinor=10000))
        assert second["result"] == "APPROVED" and held() == 10000
        assert network("/channels/atm/v1/messages", msg(token, "REVERSAL", authorizationId=second["authorizationId"]))[1]["result"] == "REVERSED" and held() == 0
        assert network("/channels/atm/v1/messages", msg(token, "COMPLETION", authorizationId=second["authorizationId"]))[1]["reason"] == "AUTHORIZATION_NOT_OPEN"
        result["reversal_and_late_reversal"] = True
        # Limits: 500.00 per withdrawal, 1,000.00 per day (200.00 already completed today).
        assert network("/channels/atm/v1/messages", msg(token, "AUTHORIZATION", operation="CASH_WITHDRAWAL", amountMinor=60000))[1]["reason"] == "LIMIT_EXCEEDED"
        _, big, _ = network("/channels/atm/v1/messages", msg(token, "AUTHORIZATION", operation="CASH_WITHDRAWAL", amountMinor=50000))
        assert big["result"] == "APPROVED"
        network("/channels/atm/v1/messages", msg(token, "COMPLETION", authorizationId=big["authorizationId"]))
        assert network("/channels/atm/v1/messages", msg(token, "AUTHORIZATION", operation="CASH_WITHDRAWAL", amountMinor=40000))[1]["reason"] == "LIMIT_EXCEEDED"
        result["limits_enforced"] = True
        # Network timeout: an uncompleted authorization expires and its hold is released by the worker.
        _, timed, _ = network("/channels/atm/v1/messages", msg(token, "AUTHORIZATION", operation="CASH_WITHDRAWAL", amountMinor=10000))
        assert timed["result"] == "APPROVED" and held() == 10000
        until = time.time() + 180
        while time.time() < until and psql(database, f"SELECT state FROM payments.atm_authorizations WHERE id='{timed['authorizationId']}'") != "EXPIRED":
            time.sleep(2)
        assert psql(database, f"SELECT state FROM payments.atm_authorizations WHERE id='{timed['authorizationId']}'") == "EXPIRED" and held() == 0
        result["timeout_expiry_releases_hold"] = True
        # Settlement reconciliation lists every difference.
        stray = str(uuid.uuid4())
        _, recon, _ = network("/channels/atm/v1/settlement-files", {"fileId": "SIM-FILE-" + uuid.uuid4().hex[:12], "businessDate": time.strftime("%Y-%m-%d", time.gmtime()),
                                                                    "completions": [{"authorizationId": auth, "amountMinor": 20000}, {"authorizationId": big["authorizationId"], "amountMinor": 45000},
                                                                                    {"authorizationId": stray, "amountMinor": 1000}]})
        outcomes = {i["authorizationId"]: i["outcome"] for i in recon["items"]}
        assert outcomes == {auth: "LATE_REVERSAL_REVIEW", big["authorizationId"]: "AMOUNT_MISMATCH", stray: "MISSING_AT_BANK"} and recon["differences"] == 3, recon
        result["settlement_reconciliation"] = {"matched": recon["matched"], "differences": recon["differences"]}
        # Common restriction and card blocking.
        assert alice.api("POST", f"/api/v1/accounts/{account}/freeze")[0] in (200, 204)
        assert network("/channels/atm/v1/messages", msg(token, "AUTHORIZATION", operation="CASH_WITHDRAWAL", amountMinor=1000))[1]["reason"] == "ACCOUNT_RESTRICTED"
        assert alice.api("POST", f"/api/v1/atm/cards/{card['id']}/block")[0] == 204
        assert network("/channels/atm/v1/messages", msg(token, "INQUIRY"))[1]["reason"] == "CARD_NOT_ACTIVE"
        result["restriction_and_blocked_card_declined"] = True
        invariants = psql(database, "SELECT (SELECT count(*) FROM (SELECT batch_id FROM core.journal_entries GROUP BY batch_id HAVING sum(delta::numeric)<>0) u)||'|'||"
                                    "(SELECT count(*) FROM core.ledger_accounts a WHERE a.booked<>coalesce((SELECT sum(delta) FROM core.journal_entries e WHERE e.account_id=a.id),0))")
        assert invariants == "0|0", invariants
        assert int(psql(database, f"SELECT booked FROM core.ledger_accounts WHERE id='{SETTLEMENT}'")) == 70000
        result.pop("_", None)
        print(json.dumps({"result": "passed", **result, "invariants": "balanced"}))
    finally:
        if fixture.poll() is None:
            os.killpg(fixture.pid, signal.SIGTERM)
            fixture.wait(timeout=60)


if __name__ == "__main__":
    main()
