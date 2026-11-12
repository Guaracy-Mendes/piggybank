#!/usr/bin/env python3
"""Record actual implementation evidence and regenerate the validated execution board."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
from implementation_plan import BACKLOG, BOARD, ROOT, load, next_package, render, validate

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("package")
parser.add_argument("status", choices=["in_progress", "done", "blocked"])
parser.add_argument("--note", default="")
parser.add_argument("--command", default="")
parser.add_argument("--artifact", default="")
parser.add_argument("--environment", default="local")
args = parser.parse_args()
data = load()
package = next((p for p in data["work_packages"] if p["id"] == args.package), None)
if package is None:
    raise SystemExit("Unknown work package")
package["status"] = args.status
data["status"] = "in_progress"
if args.status == "done":
    if not args.note or not args.command or not args.artifact or not (ROOT / args.artifact).is_file():
        raise SystemExit("Completion requires a note, actual verification command, and existing evidence artifact")
    package["completion_notes"] = args.note
    package["blocker"] = ""
    package["evidence"].append({
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "environment": args.environment,
        "command": args.command,
        "result": "passed",
        "artifact": args.artifact,
    })
elif args.status == "blocked":
    package["blocker"] = args.note
candidate = next_package(data)
data["next_work_package"] = candidate["id"] if candidate else None
validate(data)
BACKLOG.write_text(json.dumps(data, indent=2) + "\n")
BOARD.write_text(render(data))
print(f"{args.package}: {args.status}; next: {data['next_work_package']}")
