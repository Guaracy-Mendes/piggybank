#!/usr/bin/env python3
"""Check implemented router/OpenAPI provenance and generated TypeScript drift."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
SPEC=ROOT/"contracts/openapi/implemented.json"
OUTPUT=ROOT/"packages/api-client/src/schema.d.ts"
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument("--check",action="store_true")
args=parser.parse_args()
spec=json.loads(SPEC.read_text())
paths=set()
actual_methods={}
for source in [ROOT/"crates/platform/src/health.rs",ROOT/"crates/identity/src/sessions.rs",ROOT/"apps/bank-api/src/main.rs",ROOT/"apps/bank-api/src/approvals.rs",ROOT/"apps/bank-api/src/operations.rs",ROOT/"apps/bank-api/src/providers.rs",ROOT/"apps/bank-api/src/cases.rs",ROOT/"apps/bank-api/src/journeys.rs",ROOT/"apps/bank-api/src/servicing.rs",ROOT/"apps/bank-api/src/administration.rs",ROOT/"apps/bank-api/src/payments.rs",ROOT/"apps/bank-api/src/finance.rs"]:
    content=source.read_text()
    paths.update(re.findall(r'\.route\(\s*"([^"\n]+)"',content))
    for match in re.finditer(r'\.route\(\s*"([^"\n]+)"\s*,',content):
        depth=1;end=match.end()
        while end<len(content) and depth:
            if content[end]=="(":depth+=1
            elif content[end]==")":depth-=1
            end+=1
        actual_methods.setdefault(match[1],set()).update(re.findall(r'\b(get|post|put|patch|delete|head|options)\s*\(',content[match.end():end]))

if paths!=set(spec["paths"]):raise SystemExit(f"Actual router/OpenAPI path mismatch: {paths ^ set(spec['paths'])}")
for path,methods in actual_methods.items():
    declared=set(spec["paths"][path])&{"get","post","put","patch","delete","head","options"}
    if declared!=methods:raise SystemExit(f"Router/OpenAPI method mismatch at {path}: {methods ^ declared}")
subprocess.run([str(ROOT/"scripts/npm.sh"),"exec","--","node","scripts/validate_openapi.mjs"],cwd=ROOT,check=True)
with tempfile.TemporaryDirectory() as temporary:
    target=Path(temporary)/"schema.d.ts"
    subprocess.run([str(ROOT/"scripts/npm.sh"),"exec","--","openapi-typescript",str(SPEC),"--output",str(target)],cwd=ROOT,check=True)
    if args.check:
        if not OUTPUT.exists() or OUTPUT.read_text()!=target.read_text():raise SystemExit("Generated client types are stale; run make contracts.")
    else:
        OUTPUT.write_text(target.read_text())
print(f"Implemented contract and generated client {'checked' if args.check else 'updated'}: {len(paths)} actual route patterns.")
