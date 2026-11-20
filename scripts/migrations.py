#!/usr/bin/env python3
"""WP-29 migration safety: released migrations are immutable (SHA-384 lock) and new migrations are expand-only.

`lock` records every current migration. `--check` fails if a locked file changed or disappeared, if numbering
has a gap, or if an unlocked (new) migration contains destructive DDL that breaks a running N-1 binary during a
rolling upgrade. Destructive changes need an explicit later "contract" migration after all binaries moved on,
marked on the statement line with `-- contract:` and a reason.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LOCK = ROOT / "contracts/migrations.lock.json"
DESTRUCTIVE = [
    (r"\bDROP\s+TABLE\b", "drops a table"),
    (r"\bDROP\s+COLUMN\b", "drops a column"),
    (r"\bRENAME\s+(COLUMN|TO)\b", "renames an object"),
    (r"\bALTER\s+COLUMN\s+\w+\s+(SET\s+DATA\s+)?TYPE\b", "changes a column type"),
    (r"\bALTER\s+COLUMN\s+\w+\s+SET\s+NOT\s+NULL\b", "tightens nullability"),
    (r"\bTRUNCATE\b(?!\s+OR\b)", "truncates data"),
    (r"\bDROP\s+FUNCTION\b", "drops a function"),
]


def files():
    return sorted(ROOT.glob("migrations/[0-9][0-9][0-9][0-9]_*.sql"))


def digest(path):
    return hashlib.sha384(path.read_bytes()).hexdigest()


def lint(path):
    problems = []
    for number, line in enumerate(path.read_text().splitlines(), 1):
        code = line.split("--", 1)[0]
        if "-- contract:" in line:
            continue
        for pattern, why in DESTRUCTIVE:
            # Trigger declarations name TRUNCATE as an event and REVOKE/GRANT name it as a privilege, not an operation.
            if re.search(pattern, code, re.I) and not re.search(r"BEFORE\s+.*TRUNCATE|OR\s+TRUNCATE|^\s*(REVOKE|GRANT)\b", code, re.I):
                problems.append(f"{path.name}:{number} {why}")
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["lock", "check"])
    args = parser.parse_args()
    current = {p.name: digest(p) for p in files()}
    numbers = [int(name[:4]) for name in current]
    if numbers != list(range(1, len(numbers) + 1)):
        raise SystemExit("Migration numbering must be contiguous from 0001")
    if args.action == "lock":
        LOCK.write_text(json.dumps({"algorithm": "sha384", "migrations": current}, indent=2) + "\n")
        print(f"Locked {len(current)} migrations.")
        return
    locked = json.loads(LOCK.read_text())["migrations"]
    changed = [name for name, value in locked.items() if current.get(name) != value]
    if changed:
        raise SystemExit(f"Released migrations changed or removed (create a forward migration instead): {changed}")
    problems = [p for name in current if name not in locked for p in lint(ROOT / "migrations" / name)]
    if problems:
        raise SystemExit("Destructive DDL in new migrations (expand-only during rolling upgrades):\n" + "\n".join(problems))
    print(f"Migrations checked: {len(locked)} locked unchanged, {len(current) - len(locked)} new expand-only.")


if __name__ == "__main__":
    main()
