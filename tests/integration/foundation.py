#!/usr/bin/env python3
"""AT-20 foundation branch: persistent restart and bounded degraded readiness."""
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.error
import urllib.request
import urllib.parse
import uuid

ROOT = Path(__file__).resolve().parents[2]

def request(path):
    try:
        response = urllib.request.urlopen(f"http://127.0.0.1:4180{path}", timeout=4)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, json.load(response)

def ready():
    for _ in range(80):
        try:
            if request("/health/ready")[0] == 200:
                return
        except OSError:
            pass
        time.sleep(0.25)
    raise AssertionError("API did not become ready")

def issuer_login_ready():
    # Discovery can be cached while the issuer still has stale PostgreSQL connections.
    # Exercise fresh authorization forms (no credentials) for both independent realms.
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self,*args,**kwargs):return None
    opener=urllib.request.build_opener(NoRedirect())
    until=time.monotonic()+30
    for trust,realm,client in [('customer','customers','customer-web'),('workforce','workforce','staff-web')]:
        query=urllib.parse.urlencode({'client_id':client,'redirect_uri':f'http://127.0.0.1:4180/auth/{trust}/callback',
            'response_type':'code','scope':'openid','code_challenge':'a'*43,'code_challenge_method':'S256'})
        while True:
            try:
                with opener.open(f'http://127.0.0.1:18280/realms/{realm}/protocol/openid-connect/auth?'+query,timeout=3) as response:
                    if response.status==200 and b'kc-form-login' in response.read(65537):break
            except (OSError,urllib.error.HTTPError):pass
            if time.monotonic()>=until:raise AssertionError('Issuer login-form readiness failed after PostgreSQL restart')
            time.sleep(.25)

def sql(statement):
    return subprocess.check_output([os.environ.get("CONTAINER_RUNTIME", "podman"), "exec", "piggybank-postgres-1", "psql", "-U", "dev_admin", "-d", "piggybank", "-At", "-v", "ON_ERROR_STOP=1", "-c", statement], text=True).strip()

def main():
    runtime = os.environ.get("CONTAINER_RUNTIME", "podman")
    process = subprocess.Popen([str(ROOT / "target/debug/bank-api")], cwd=ROOT, stdout=subprocess.DEVNULL)
    paused = False
    try:
        ready()
        sentinel = str(uuid.uuid4())
        sql("CREATE SCHEMA IF NOT EXISTS foundation_test; CREATE TABLE IF NOT EXISTS foundation_test.probe(id uuid PRIMARY KEY)")
        sql(f"INSERT INTO foundation_test.probe VALUES ('{sentinel}')")
        subprocess.run([runtime, "restart", "piggybank-postgres-1"], check=True, stdout=subprocess.DEVNULL)
        ready()
        issuer_login_ready()
        assert sql(f"SELECT count(*) FROM foundation_test.probe WHERE id='{sentinel}'") == "1"
        subprocess.run([runtime, "pause", "piggybank-nats-1"], check=True, stdout=subprocess.DEVNULL)
        paused = True
        started = time.monotonic()
        status, body = request("/health/ready")
        elapsed = time.monotonic() - started
        assert status == 503 and body["dependencies"]["jetstream"] is False
        assert elapsed < 3, elapsed
        assert request("/health/live")[0] == 200
        subprocess.run([runtime, "unpause", "piggybank-nats-1"], check=True, stdout=subprocess.DEVNULL)
        paused = False
        ready()
        print(json.dumps({"result": "passed", "persistent_postgres_restart": True, "readiness_timeout_seconds": round(elapsed, 3), "liveness_during_broker_outage": True, "issuer_login_forms_after_database_restart": True}))
    finally:
        if paused:
            subprocess.run([runtime, "unpause", "piggybank-nats-1"], check=True, stdout=subprocess.DEVNULL)
        process.terminate()
        process.wait(timeout=10)
        sql("DROP SCHEMA IF EXISTS foundation_test CASCADE")

if __name__ == "__main__":
    main()
