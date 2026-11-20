#!/usr/bin/env python3
"""Local CI-equivalent pipeline (WP-29). No remote CI host is configured: a pipeline definition is not evidence of a
remote run. Steps run sequentially (shared PostgreSQL/browser suites cannot overlap) and stop at the first failure
of a gating stage. Evidence goes to `.local/ci/<UTC timestamp>/`: per-step logs plus a sanitized `evidence.json`
holding commands, exit codes, durations, tool versions and the source revision. No environment values or secrets.

  python3 scripts/ci.py                 # static + disposable-database + isolated browser/mobile suites
  python3 scripts/ci.py --apply-local   # also migrate the persistent local database, then port-bound suites
  python3 scripts/ci.py --device        # also run the Android emulator journey (needs the SDK and an APK)
  python3 scripts/ci.py --load          # also run the release-build load smoke (informational, WP-30)
"""
import argparse
import datetime
import json
import os
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DB = "postgres://dev_admin:synthetic-admin-only@127.0.0.1:15432/piggybank"
STAGES = [
    ("static", [
        ("app-check", ["make", "app-check"]),
        ("web-build", ["make", "web-build"]),
        ("ui-sources", ["make", "ui-sources"]),
        ("docs", ["make", "docs"]),
        ("plan-and-models", ["make", "check"]),
        ("migrations", ["python3", "scripts/migrations.py", "check"]),
        ("acceptance-manifest", ["python3", "scripts/acceptance.py"]),
        ("mobile-types", ["scripts/mobile.sh", "check"]),
    ]),
    ("disposable-database", [
        ("integration", ["scripts/cargo.sh", "test", "--workspace", "--locked", "--test", "database", "--", "--test-threads=2"]),
        ("unit", ["scripts/cargo.sh", "test", "--workspace", "--locked", "--lib"]),
        ("api-binary", ["scripts/cargo.sh", "test", "--locked", "-p", "bank-api", "--bin", "bank-api"]),
        ("build", ["scripts/cargo.sh", "build", "--workspace", "--locked"]),
    ]),
    ("isolated-runtime", [
        ("config-failure", ["python3", "tests/integration/deployment/config_failure.py"]),
        ("operations-drills", ["python3", "tests/integration/deployment/operations.py"]),
        ("native-contract", ["python3", "tests/mobile/native_api.py"]),
        ("issuer-identity", ["python3", "tests/integration/identity.py"]),
        ("security", ["python3", "tests/acceptance/security/security.py"]),
        ("resilience-drills", ["python3", "tests/resilience/drills.py"]),
        ("partner-consents", ["python3", "tests/acceptance/partners/partners.py"]),
        ("assisted-channels", ["python3", "tests/acceptance/assisted/assisted.py"]),
        ("atm-channel", ["python3", "tests/acceptance/atm/atm.py"]),
        ("batch-rail", ["python3", "tests/acceptance/batch/batch.py"]),
        ("governed-warehouse", ["python3", "tests/acceptance/analytics/analytics.py"]),
        ("audiences-journeys", ["python3", "tests/acceptance/journeys/journeys.py"]),
        ("payment-routing", ["python3", "tests/acceptance/routing/routing.py"]),
        ("liquidity-forecasting", ["python3", "tests/acceptance/liquidity/liquidity.py"]),
        ("outcome-kpis", ["python3", "tests/acceptance/outcomes/outcomes.py"]),
        ("consumer-browser", ["scripts/npm.sh", "exec", "--", "playwright", "test", "--config", "playwright.consumer.config.ts"]),
    ]),
]
LOCAL = ("persistent-local", [
    ("preview-stop", ["python3", "scripts/local_cx.py", "stop"]),
    ("migrate-persistent", ["python3", "scripts/local_cx.py", "prepare"]),
    ("preview-stop-after-prepare", ["python3", "scripts/local_cx.py", "stop"]),
    ("payments-browser", ["scripts/npm.sh", "exec", "--", "playwright", "test", "--config", "playwright.payments.config.ts"]),
    ("customer-browser", ["scripts/npm.sh", "exec", "--", "playwright", "test", "--config", "playwright.customer.config.ts"]),
    ("staff-browser", ["scripts/npm.sh", "exec", "--", "playwright", "test", "--config", "playwright.staff.config.ts"]),
    ("issuer-browser", ["scripts/npm.sh", "exec", "playwright", "test"]),
    ("ingress", ["python3", "tests/integration/ingress.py"]),
])
LOAD = ("load-qualification", [
    ("release-build", ["scripts/cargo.sh", "build", "--release", "--locked", "-p", "bank-api", "-p", "bank-worker", "-p", "provider-simulator"]),
    ("load-smoke", ["python3", "tests/load/run.py", "--sustained", "60", "--burst", "30", "--core-seconds", "15"]),
])  # run.py fails only on broken financial invariants; NFR checks are recorded in its report, never gated here
DEVICE = ("android-device", [("android-journey", ["python3", "tests/mobile/device.py"])])


def version(command):
    try:
        return subprocess.run(command, capture_output=True, text=True, timeout=30, cwd=ROOT).stdout.strip().splitlines()[0]
    except Exception:
        return "unavailable"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--apply-local", action="store_true")
    parser.add_argument("--device", action="store_true")
    parser.add_argument("--load", action="store_true", help="also run the release-build load smoke (WP-30)")
    args = parser.parse_args()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out = ROOT / ".local/ci" / stamp
    out.mkdir(parents=True)
    stages = list(STAGES) + ([LOCAL] if args.apply_local else []) + ([LOAD] if args.load else []) + ([DEVICE] if args.device else [])
    env = dict(os.environ, BANK_TEST_ADMIN_URL=os.environ.get("BANK_TEST_ADMIN_URL", DB))
    evidence = {
        "pipeline": "local-ci-equivalent",
        "remoteCi": "not configured (no chosen host)",
        "startedAt": stamp,
        "revision": version(["git", "rev-parse", "HEAD"]),
        "uncommittedPaths": len(subprocess.run(["git", "status", "--porcelain"], capture_output=True, text=True, cwd=ROOT).stdout.splitlines()),
        "migrations": len(list((ROOT / "migrations").glob("[0-9]*.sql"))),
        "tools": {"rustc": version(["rustc", "--version"]), "node": version(["scripts/npm.sh", "exec", "--", "node", "--version"]), "podman": version(["podman", "--version"])},
        "steps": [],
        "result": "passed",
    }
    for stage, steps in stages:
        for name, command in steps:
            started = time.monotonic()
            with open(out / f"{name}.log", "w") as log:
                rc = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
            evidence["steps"].append({"stage": stage, "step": name, "command": " ".join(command), "exitCode": rc, "seconds": round(time.monotonic() - started, 1)})
            print(f"{stage:20} {name:28} rc={rc} {evidence['steps'][-1]['seconds']}s", flush=True)
            if rc != 0:
                evidence["result"] = "failed"
                break
        if evidence["result"] != "passed":
            break
    evidence["finishedAt"] = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    (out / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(f"Evidence: {out.relative_to(ROOT)}/evidence.json result={evidence['result']}")
    raise SystemExit(0 if evidence["result"] == "passed" else 1)


if __name__ == "__main__":
    main()
