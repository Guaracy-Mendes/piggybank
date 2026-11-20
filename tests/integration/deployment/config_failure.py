#!/usr/bin/env python3
"""WP-29 protected runtime configuration: unsafe or incomplete configuration stops startup; metrics need a token."""
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tests/integration"))
from database_fixture import Database  # noqa: E402

API = ROOT / "target/debug/bank-api"
WORKER = ROOT / "target/debug/bank-worker"


def refused(binary, env, expect):
    r = subprocess.run([str(binary)], env=env, capture_output=True, text=True, timeout=60)
    output = r.stdout + r.stderr
    assert r.returncode != 0, (binary.name, "started with unsafe configuration")
    assert expect in output, (expect, output[-400:])
    return r.returncode


def get(path, headers=None):
    try:
        with urllib.request.urlopen(urllib.request.Request("http://127.0.0.1:4196" + path, headers=headers or {}), timeout=10) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def main():
    db = Database()
    tmp = Path(tempfile.mkdtemp(prefix="config-", dir=ROOT / ".local"))
    process = None
    try:
        base = dict(db.env, BANK_BIND="127.0.0.1:4196")
        refused(API, dict(base, BANK_PROFILE="production"), "Only the synthetic local profile")
        refused(WORKER, dict(base, BANK_PROFILE="production", BANK_BIND="127.0.0.1:4197"), "Only the synthetic local profile")
        bad_keys = tmp / "keys"
        bad_keys.write_text("not-a-key-line\n")
        refused(API, dict(base, BANK_SESSION_KEYS_FILE=str(bad_keys)), "BANK_SESSION_KEYS_FILE")
        token = tmp / "metrics.token"
        token.write_text("m" * 48)
        token.chmod(0o644)
        refused(API, dict(base, BANK_METRICS_TOKEN_FILE=str(token)), "must not be readable by group or others")
        short = tmp / "admin.secret"
        short.write_text("short")
        short.chmod(0o600)
        refused(API, dict(base, BANK_IDENTITY_ADMIN_SECRET_FILE=str(short)), "BANK_IDENTITY_ADMIN_SECRET_FILE")
        refused(API, dict(base, BANK_IDENTITY_ADMIN_SECRET_FILE=str(tmp / "missing")), "unreadable")
        token.chmod(0o600)
        good_keys = tmp / "session.keys"
        good_keys.write_text("k1:" + __import__("base64").b64encode(os.urandom(32)).decode() + "\n")
        good_keys.chmod(0o600)
        process = subprocess.Popen([str(API)], env=dict(base, BANK_METRICS_TOKEN_FILE=str(token), BANK_SESSION_KEYS_FILE=str(good_keys)),
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(60):
            try:
                if get("/health/ready")[0] == 200:
                    break
            except OSError:
                pass
            time.sleep(0.25)
        else:
            raise AssertionError("API readiness timeout")
        status, body = get("/metrics")
        assert status == 401 and "m" * 48 not in body
        status, body = get("/metrics", {"Authorization": "Bearer " + "m" * 48})
        assert status == 200, status
        for name in ["bank_http_requests_total", "bank_http_request_duration_seconds_bucket", "bank_outbox_undelivered", "bank_unbalanced_journal_batches 0", "bank_pages_unacknowledged"]:
            assert name in body, name
        assert 'route="/health/ready"' in body
        print(json.dumps({"result": "passed", "production_profile_refused": True, "worker_production_refused": True, "malformed_session_keys_refused": True,
                          "world_readable_secret_refused": True, "short_or_missing_secret_refused": True, "metrics_token_required": True, "metrics_exposed": True}))
    finally:
        if process:
            process.terminate()
            process.wait(timeout=20)
        for p in tmp.glob("*"):
            p.unlink()
        tmp.rmdir()
        db.close()


if __name__ == "__main__":
    main()
