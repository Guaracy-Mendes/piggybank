#!/usr/bin/env python3
"""Validate the acceptance manifest (WP-30) and print branch-level results.

Every R1-applicable scenario (AT-01..AT-20, AT-24..AT-30) must be present. Every PASS branch must reference
evidence that exists in the repository: a Rust test function, a Playwright test title, or a Python suite result key.
PENDING and NOT_APPLICABLE_R1 branches must state a reason. A FAIL branch records a measured shortfall: it needs both
the evidence that measured it and a reason. FAIL keeps the manifest valid (it is an honest result), but `--strict`
exits non-zero while any branch fails. Nothing is silently skipped.
"""
import json
import re
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "tests/acceptance/manifest.json"


def exists(reference):
    path, _, name = reference.partition("::")
    file = ROOT / path
    if not file.exists():
        return False
    text = file.read_text()
    if path.endswith(".rs"):
        return re.search(rf"fn {re.escape(name)}\s*\(", text) is not None
    if path.endswith(".ts"):
        return f'test("{name}"' in text or f"test(\n  \"{name}\"" in text or name in text
    return name in text  # Python suites: result keys printed in their JSON summary


def main():
    data = json.loads(MANIFEST.read_text())
    ids = {s["id"] for s in data["scenarios"]}
    required = {f"AT-{i:02d}" for i in list(range(1, 21)) + list(range(24, 31))}
    problems = [f"missing scenario {x}" for x in sorted(required - ids)]
    counts = Counter()
    rows = []
    for scenario in data["scenarios"]:
        for branch in scenario["branches"]:
            status = branch["status"]
            counts[status] += 1
            if status not in {"PASS", "FAIL", "PENDING", "NOT_APPLICABLE_R1"}:
                problems.append(f"{scenario['id']}/{branch['id']}: unknown status {status}")
            if status in {"PASS", "FAIL"}:
                if not branch.get("evidence"):
                    problems.append(f"{scenario['id']}/{branch['id']}: {status} without evidence")
                for ref in branch.get("evidence", []):
                    if not exists(ref):
                        problems.append(f"{scenario['id']}/{branch['id']}: evidence not found: {ref}")
            if status != "PASS" and not branch.get("reason"):
                problems.append(f"{scenario['id']}/{branch['id']}: {status} without a reason")
            if status == "NOT_APPLICABLE_R1" and scenario["id"] in required and branch["id"] == "all":
                problems.append(f"{scenario['id']}: R1 scenario cannot be wholly not applicable")
            rows.append((scenario["id"], branch["id"], status))
    if problems:
        raise SystemExit("Acceptance manifest invalid:\n" + "\n".join(problems))
    for row in rows:
        print(f"{row[0]}  {row[2]:18} {row[1]}")
    if "--strict" in sys.argv and counts["FAIL"]:
        raise SystemExit(f"Acceptance manifest valid but {counts['FAIL']} branch(es) FAIL (--strict)")
    print(f"Acceptance manifest valid: {len(ids)} scenarios, {sum(counts.values())} branches — " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))


if __name__ == "__main__":
    sys.exit(main())
