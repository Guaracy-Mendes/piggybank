#!/usr/bin/env python3
"""WP-37 governed warehouse (one local adapter) acceptance against the real local issuer, API and worker.

Proves through the staff API: a snapshot taken before ingestion is flagged INCOMPLETE; a backfill ingests the projected
Core postings once and a second backfill only finds duplicates; the next snapshot reconciles with Core, supersedes the
first and traces each aggregate row to its source rows, schema version and ingestion run; it reproduces from its stored
cuts. A marketing request is refused with 403 and recorded. An incompatible schema version is refused with 409 and an
additive optional field is accepted; new facts record version 2 while earlier facts keep version 1. No analytics call
changes a Core balance or journal.
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


def psql(database, statement):
    return subprocess.check_output(["podman", "exec", "piggybank-postgres-1", "psql", "-U", "dev_admin", "-d", database, "-At", "-v", "ON_ERROR_STOP=1", "-c", statement], text=True).strip()


def core_state(database):
    return psql(database, "SELECT (SELECT count(*) FROM core.journal_entries)||'|'||(SELECT coalesce(sum(booked),0) FROM core.ledger_accounts)||'|'||"
                          "(SELECT coalesce(sum(abs(booked)),0) FROM core.ledger_accounts)||'|'||(SELECT coalesce(sum(held),0) FROM core.ledger_accounts)")


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
        account = n.open_account(alice, "Analyticsalice")
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), account, "123400"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        # The worker projects Core events into Data asynchronously.
        for _ in range(120):
            if psql(database, f"SELECT count(*) FROM data.transaction_views WHERE account_id='{account}'") == "1":
                break
            time.sleep(0.5)
        ops, csrf = security.workforce_login("reviewer")
        api = lambda method, path, body=None: security.staff(ops, csrf, method, path, body)  # noqa: E731
        today = psql(database, "SELECT (now() AT TIME ZONE 'UTC')::date")
        request = {"transform": "daily_ledger_movement", "version": 1, "purpose": "FINANCIAL_REPORTING", "from": today, "to": today}
        before = core_state(database)
        result = {}
        # The worker ingests every 30 s; take the first snapshot before it has seen this posting by checking its flag.
        status, first = api("POST", "/api/v1/staff/analytics/snapshots", request)
        assert status == 200 and first["outcome"] == "CREATED", (status, first)
        assert first["status"] == "INCOMPLETE" and first["control"]["reconciled"] is False, first["completeness"]
        result["missing_or_stale_sources_flagged"] = True
        window = {"from": (datetime.now(timezone.utc) - timedelta(hours=6)).isoformat(), "to": datetime.now(timezone.utc).isoformat()}
        for dataset in ("account_states", "ledger_entries"):
            status, run = api("POST", "/api/v1/staff/analytics/ingestions", {"dataset": dataset, **window})
            assert status == 200 and run["mode"] == "BACKFILL", (status, run)
        status, rerun = api("POST", "/api/v1/staff/analytics/ingestions", {"dataset": "ledger_entries", **window})
        assert status == 200 and rerun["inserted"] == 0 and rerun["duplicates"] >= 2 and rerun["runId"], rerun
        result["backfill_replay_dedups_economically"] = True
        status, snap = api("POST", "/api/v1/staff/analytics/snapshots", request)
        assert status == 200 and snap["status"] == "COMPLETE" and snap["control"]["reconciled"] is True, snap["completeness"]
        assert snap["supersedes"] == first["id"] and snap["purpose"] == "FINANCIAL_REPORTING" and snap["adapter"] == "LOCAL_WAREHOUSE"
        ledger = next(i for i in snap["inputs"] if i["dataset"] == "ledger_entries")
        assert ledger["schemaVersions"] == [1] and ledger["runs"]
        funding = next(r for r in snap["rows"] if r["dimensions"]["postingKind"] == "SYNTHETIC_FUNDING" and r["creditMinor"] != "0" and not r["dimensions"]["partyRef"].startswith("INTERNAL"))
        status, trace = api("GET", f"/api/v1/staff/analytics/snapshots/{snap['id']}/rows/{funding['rowNo']}/trace")
        assert status == 200 and trace["purpose"] == "FINANCIAL_REPORTING" and trace["transformDigest"] == snap["transformDigest"]
        assert any(f["sourceRef"].startswith("transaction_views:") and f["schemaVersion"] == 1 and f["runId"] in ledger["runs"] for f in trace["facts"]), trace
        status, reproduced = api("POST", f"/api/v1/staff/analytics/snapshots/{snap['id']}/reproduction")
        assert status == 200 and reproduced["reproduced"] is True
        result["aggregate_traces_to_source_schema_transform_purpose"] = True
        # Marketing use: refused and recorded (the reason is in the recorded denials).
        status, _ = api("POST", "/api/v1/staff/analytics/snapshots", dict(request, purpose="MARKETING"))
        assert status == 403
        status, view = api("GET", "/api/v1/staff/analytics")
        assert status == 200 and view["denials"][0]["reason"] == "PURPOSE_NOT_PERMITTED_FOR_DATASET" and view["denials"][0]["purpose"] == "MARKETING"
        result["purpose_denial_recorded"] = True
        # Schema upgrade: removal refused, additive optional field accepted, older facts keep version 1.
        v1 = next(d for d in view["datasets"] if d["code"] == "ledger_entries")["schemas"][0]["fields"]
        status, _ = api("POST", "/api/v1/staff/analytics/schemas", {"dataset": "ledger_entries", "version": 2, "fields": [f for f in v1 if f["name"] != "currency"]})
        assert status == 409
        status, upgraded = api("POST", "/api/v1/staff/analytics/schemas", {"dataset": "ledger_entries", "version": 2,
                                                                          "fields": v1 + [{"name": "productVersionId", "type": "uuid", "required": False}]})
        assert status == 200 and upgraded["compatibility"] == "BACKWARD", upgraded
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), account, "100"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        for _ in range(120):
            if psql(database, f"SELECT count(*) FROM data.transaction_views WHERE account_id='{account}'") == "2":
                break
            time.sleep(0.5)
        status, run = api("POST", "/api/v1/staff/analytics/ingestions", {"dataset": "ledger_entries", "from": window["from"], "to": datetime.now(timezone.utc).isoformat()})
        assert status == 200 and run["schemaVersion"] == 2 and run["inserted"] == 2, run
        assert psql(database, "SELECT string_agg(schema_version||':'||n,',' ORDER BY schema_version) FROM (SELECT schema_version,count(*) n FROM analytics.facts WHERE dataset='ledger_entries' GROUP BY 1) x").startswith("1:")
        status, v2 = api("POST", "/api/v1/staff/analytics/snapshots", dict(request, version=2))
        assert status == 200 and next(i for i in v2["inputs"] if i["dataset"] == "ledger_entries")["schemaVersions"] == [1, 2], v2["inputs"]
        result["schema_upgrade_backward_compatible"] = True
        # Analytics never changed Core (the second funding is the only Core change, made outside analytics).
        after = core_state(database)
        assert after.split("|")[0] == str(int(before.split("|")[0]) + 2) and after.split("|")[1] == before.split("|")[1], (before, after)
        assert psql(database, "SELECT has_schema_privilege('bank_data','core','USAGE')") == "f"
        result["analytics_cannot_change_core"] = True
        print(json.dumps({"result": "passed", **result}))
    finally:
        if fixture.poll() is None:
            os.killpg(fixture.pid, signal.SIGTERM)
            fixture.wait(timeout=60)


if __name__ == "__main__":
    main()
