#!/usr/bin/env python3
"""WP-29 operations drills on the isolated fixture (real API, worker, provider and issuer).

1. NFR-10 alert drill: an injected projection gap pages the on-call primary within one minute through the
   worker's data-quality scan and paging tick; the paged person acknowledges.
2. FR-OPS-04 backup/restore drill: pg_dump/pg_restore with measured durations, ledger invariants and migration
   checksums verified on the restored copy, and an API on the restored database serving the same balance
   before any financial write resumes.
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

_spec = importlib.util.spec_from_file_location("security_acceptance", ROOT / "tests/acceptance/security/security.py")
security = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(security)


def psql(database, statement):
    return subprocess.check_output(["podman", "exec", "piggybank-postgres-1", "psql", "-U", "dev_admin", "-d", database, "-At", "-v", "ON_ERROR_STOP=1", "-c", statement], text=True).strip()


def invariants(database, exclude):
    return {
        "migrations": psql(database, "SELECT count(*)||':'||md5(string_agg(encode(checksum,'hex'),'' ORDER BY version)) FROM _sqlx_migrations"),
        "unbalancedBatches": int(psql(database, "SELECT count(*) FROM (SELECT batch_id FROM core.journal_entries GROUP BY batch_id HAVING sum(delta::numeric)<>0) u")),
        # The drill's injected gap batch deliberately bypassed Core's balance update; it is excluded here.
        "bookedMismatches": int(psql(database, f"SELECT count(*) FROM core.ledger_accounts a WHERE a.booked<>coalesce((SELECT sum(delta) FROM core.journal_entries e WHERE e.account_id=a.id AND e.batch_id<>'{exclude}'),0)")),
        "heldMismatches": int(psql(database, "SELECT count(*) FROM core.ledger_accounts a WHERE a.held<>coalesce((SELECT sum(amount) FROM core.holds h WHERE h.account_id=a.id AND h.state='ACTIVE'),0)")),
        "journalBatches": int(psql(database, "SELECT count(*) FROM core.journal_batches")),
        "payments": psql(database, "SELECT count(*)||':'||coalesce(string_agg(state,',' ORDER BY id),'') FROM payments.intents"),
        "auditEvents": int(psql(database, "SELECT count(*) FROM controls.audit_events")),
        "auditChainHeads": psql(database, "SELECT coalesce(md5(string_agg(stream||sequence||digest,'' ORDER BY stream)),'') FROM controls.audit_heads"),
    }


def main():
    env = dict(os.environ, BANK_BROWSER_RATE_LIMIT="5000")
    fixture = subprocess.Popen(["python3", str(ROOT / "tests/browser-customer/server.py")], cwd=ROOT, env=env, start_new_session=True, stdout=subprocess.DEVNULL)
    restored = None
    api = None
    try:
        for _ in range(240):
            try:
                if n.http("GET", n.API + "/health/ready")[0] == 200 and n.http("GET", "http://127.0.0.1:4301")[0] == 200:
                    break
            except OSError:
                pass
            time.sleep(0.5)
        database = json.loads((ROOT / ".local/customer-browser-fixture.json").read_text())["database"]
        alice, bob = n.Session("alice"), n.Session("bob")
        source, target = n.open_account(alice, "Drill"), n.open_account(bob, "Drillrecipient")
        subprocess.run([str(ROOT / "target/debug/bank-fund"), str(uuid.uuid4()), source, "50000"], check=True, stdout=subprocess.DEVNULL,
                       env=dict(os.environ, BANK_PROFILE="local", BANK_CORE_DATABASE_URL=f"postgres://bank_core:synthetic-core-only@127.0.0.1:15432/{database}"))
        # --- NFR-10 alert drill -------------------------------------------------------------------------
        psql(database, "UPDATE data.quality_thresholds SET seconds=0 WHERE check_code IN ('MISSING_FINANCIAL_EVENT','STALE_PROJECTION')")
        gap = str(uuid.uuid4())
        injected_at = float(psql(database, f"""DO $$ BEGIN
          INSERT INTO core.journal_batches(id,operation_id,posting_kind,fingerprint) VALUES('{gap}','{gap}','SYNTHETIC_FUNDING','drill');
          INSERT INTO core.journal_entries VALUES('{gap}','{source}',1),('{gap}','4f1a780b-3e31-48ef-a294-000000000003',-1); END $$;
          SELECT extract(epoch FROM created_at) FROM core.journal_batches WHERE id='{gap}'""").splitlines()[-1])
        page = None
        for _ in range(120):
            row = psql(database, f"""SELECT p.id||'|'||extract(epoch FROM p.delivered_at)||'|'||coalesce(d.tier,'') FROM controls.pages p JOIN data.quality_alerts q ON q.id=p.source_id
              LEFT JOIN controls.page_deliveries d ON d.page_id=p.id WHERE q.check_code='MISSING_FINANCIAL_EVENT' AND q.subject='{gap}' AND p.delivered_at IS NOT NULL""")
            if row:
                page = row.split("|")
                break
            time.sleep(1)
        assert page, "no page delivered for the injected projection gap"
        delivery_seconds = float(page[1]) - injected_at
        assert delivery_seconds <= 60, ("NFR-10 delivery objective exceeded", delivery_seconds)
        assert page[2] == "PRIMARY"
        reviewer, csrf = security.workforce_login("reviewer")
        status, ack = security.staff(reviewer, csrf, "POST", f"/api/v1/staff/pages/{page[0]}/acknowledgement")
        assert status == 200 and ack["acknowledged"] is True, (status, ack)
        acked = float(psql(database, f"SELECT extract(epoch FROM acknowledged_at-delivered_at) FROM controls.pages WHERE id='{page[0]}'"))
        # --- FR-OPS-04 backup / restore drill -------------------------------------------------------------
        before = invariants(database, gap)
        before.pop("unbalancedBatches")
        dump = f"/tmp/{database}.dump"
        started = time.monotonic()
        subprocess.run(["podman", "exec", "piggybank-postgres-1", "pg_dump", "-U", "dev_admin", "-Fc", "-f", dump, database], check=True)
        dump_seconds = time.monotonic() - started
        restored = "piggybank_http_test_" + uuid.uuid4().hex
        psql("postgres", f"CREATE DATABASE {restored} OWNER bank_migrator")
        started = time.monotonic()
        subprocess.run(["podman", "exec", "piggybank-postgres-1", "pg_restore", "-U", "dev_admin", "--exit-on-error", "-d", restored, dump], check=True)
        restore_seconds = time.monotonic() - started
        subprocess.run(["podman", "exec", "piggybank-postgres-1", "rm", "-f", dump], check=True)
        after = invariants(restored, gap)
        assert after["unbalancedBatches"] == 0 and after["bookedMismatches"] == 0 and after["heldMismatches"] == 0, after
        after.pop("unbalancedBatches")
        assert after == before, ("restored copy differs", before, after)
        # The restored database serves the same authoritative balance before financial writes resume.
        restored_env = dict(os.environ, BANK_BIND="127.0.0.1:4198", BANK_EVENT_NAMESPACE="bank.restore_" + restored[-12:], BANK_EVENT_STREAM="BANK_RESTORE_" + restored[-12:].upper())
        for module in ["platform", "identity", "core", "controls", "journeys", "payments", "finance", "documents", "data", "simulator"]:
            restored_env["BANK_" + module.upper() + "_DATABASE_URL"] = f"postgres://bank_{module}:synthetic-{module}-only@127.0.0.1:15432/{restored}"
        restored_env["DATABASE_URL"] = restored_env["BANK_PLATFORM_DATABASE_URL"]
        api = subprocess.Popen([str(ROOT / "target/debug/bank-api")], env=restored_env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(80):
            try:
                if n.http("GET", "http://127.0.0.1:4198/health/ready")[0] == 200:
                    break
            except OSError:
                pass
            time.sleep(0.25)
        else:
            raise AssertionError("restored API readiness timeout")
        live = alice.api("GET", f"/api/v1/accounts/{source}/balance", expect=200)[1]
        status, copy = n.http("GET", f"http://127.0.0.1:4198/api/v1/accounts/{source}/balance", headers={"Authorization": "Bearer " + alice.tokens["access_token"]})
        if status == 401:
            alice.refresh()
            status, copy = n.http("GET", f"http://127.0.0.1:4198/api/v1/accounts/{source}/balance", headers={"Authorization": "Bearer " + alice.tokens["access_token"]})
        assert status == 200 and copy["booked"] == live["booked"] and copy["available"] == live["available"], (status, copy, live)
        print(json.dumps({"result": "passed", "page_delivery_seconds": round(delivery_seconds, 1), "page_tier": "PRIMARY", "acknowledged_after_seconds": round(acked, 1),
                          "dump_seconds": round(dump_seconds, 2), "restore_seconds": round(restore_seconds, 2), "invariants_equal": True,
                          "journal_batches": after["journalBatches"], "restored_api_balance_matches": True}))
    finally:
        if api:
            api.terminate()
            api.wait(timeout=20)
        if restored:
            psql("postgres", f"DROP DATABASE IF EXISTS {restored} WITH (FORCE)")
        os.killpg(fixture.pid, signal.SIGTERM)
        fixture.wait(timeout=60)


if __name__ == "__main__":
    main()
