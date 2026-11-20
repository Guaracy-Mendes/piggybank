#!/usr/bin/env python3
"""Verify the user-designated UI sources and explicit 32-screen scope record."""
import hashlib
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
d=json.loads((ROOT/'docs/implementation/ui/backlog.json').read_text())
for name,expected in d['source_files'].items():
    actual=hashlib.sha256((ROOT/name).read_bytes()).hexdigest()
    if actual!=expected:raise SystemExit(f'Official source changed: {name}; reconcile its provenance explicitly.')
assert sorted(s['prd_screen'] for s in d['screens'])==list(range(1,33))
assert len({s['id'] for s in d['screens']})==32
for s in d['screens']:
    assert s['status'] in {'local_implemented','local_partial','pending_provider','pending_device'}
    assert s['implemented'] and s['route'] and s['qualification_or_remaining']
for item in json.loads((ROOT/'packages/web-core/src/fonts/provenance.json').read_text()):
    path=ROOT/'packages/web-core/src/fonts'/item['file']
    assert hashlib.sha256(path.read_bytes()).hexdigest()==item['sha256']
print('Six official sources unchanged; all 32 PRD screens explicitly mapped; four font assets match retained provenance.')
