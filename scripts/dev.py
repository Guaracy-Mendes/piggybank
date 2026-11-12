#!/usr/bin/env python3
"""Manage only the isolated piggybank local development project."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
COMPOSE = [os.environ.get("CONTAINER_RUNTIME", "podman"), "compose", "-p", "piggybank", "-f", str(ROOT / "infra/compose/compose.yaml")]

def wait_url(url: str, seconds: int = 120) -> dict:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=3) as response:
                return json.load(response)
        except (OSError, ValueError):
            time.sleep(0.5)
    raise SystemExit(f"Timed out waiting for {url}")

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["up", "down", "status", "run", "smoke"])
    args = parser.parse_args()
    if args.action == "up":
        subprocess.run(COMPOSE + ["up", "-d"], cwd=ROOT, check=True)
        wait_url("http://127.0.0.1:18280/realms/customers/.well-known/openid-configuration")
        wait_url("http://127.0.0.1:18222/healthz")
        print("Persistent local PostgreSQL, Keycloak and JetStream are ready.")
    elif args.action == "down":
        subprocess.run(COMPOSE + ["down"], cwd=ROOT, check=True)
        print("Project stopped; persistent volumes retained.")
    elif args.action == "status":
        subprocess.run(COMPOSE + ["ps"], cwd=ROOT, check=True)
    elif args.action in {"run", "smoke"}:
        subprocess.run([str(ROOT / "scripts/cargo.sh"), "build", "--workspace", "--locked"], check=True, cwd=ROOT)
        runtime_env=dict(os.environ)
        if args.action=="run":
            runtime_env.setdefault("BANK_CUSTOMER_WEB_ORIGIN","http://127.0.0.1:4300")
            runtime_env.setdefault("BANK_WORKFORCE_WEB_ORIGIN","http://127.0.0.1:4301")
        processes=[subprocess.Popen([str(ROOT/f"target/debug/{name}")],cwd=ROOT,env=runtime_env,start_new_session=True) for name in ["bank-api","bank-worker","provider-simulator"]]
        try:
            if args.action == "run":
                def stop(*_):
                    raise KeyboardInterrupt
                signal.signal(signal.SIGTERM, stop)
                wait_url("http://127.0.0.1:4180/health/ready",15)
                processes.append(subprocess.Popen([str(ROOT/"scripts/npm.sh"),"run","dev","--workspace","@piggybank/customer-web"],cwd=ROOT,env=runtime_env,start_new_session=True))
                processes.append(subprocess.Popen([str(ROOT/"scripts/npm.sh"),"run","dev","--workspace","@piggybank/staff-web"],cwd=ROOT,env=runtime_env,start_new_session=True))
                print("Staff workbench: http://127.0.0.1:4301 · Customer portal: http://127.0.0.1:4300 · API: http://127.0.0.1:4180",flush=True)
                while all(p.poll() is None for p in processes):
                    time.sleep(0.5)
            else:
                for port in [4180, 4181]:
                    live = wait_url(f"http://127.0.0.1:{port}/health/live", 10)
                    ready = wait_url(f"http://127.0.0.1:{port}/health/ready", 15)
                    assert live["status"] == "alive"
                    assert ready["status"] == "ready" and all(ready["dependencies"].values())
                print("API and worker liveness/readiness passed with real local dependencies.")
        except KeyboardInterrupt:
            pass
        finally:
            for p in processes:
                try:os.killpg(p.pid,signal.SIGTERM)
                except ProcessLookupError:pass
            for p in processes:
                try:
                    p.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    try:os.killpg(p.pid,signal.SIGKILL)
                    except ProcessLookupError:pass
                    p.wait()

if __name__ == "__main__":
    main()
