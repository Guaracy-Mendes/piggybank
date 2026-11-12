#!/usr/bin/env python3
"""Pack independently supplied synthetic provider rows; never reads bank state or publishes files."""
import argparse
from datetime import date, datetime, timezone
import json
import os
from pathlib import Path
import re
import shutil
import uuid


def encoded(value):
    return (json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False) + '\n').encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('rows', type=Path, help='JSON array exported independently by the provider')
    parser.add_argument('output', type=Path, help='New private directory; existing directories are refused')
    parser.add_argument('--kind', choices=('outgoing', 'credits'), required=True)
    parser.add_argument('--from-date', type=date.fromisoformat, required=True)
    parser.add_argument('--to-date', type=date.fromisoformat, required=True)
    parser.add_argument('--cutoff', required=True, help='Explicit timezone-aware provider snapshot cutoff')
    args = parser.parse_args()
    if os.environ.get('BANK_PROFILE', 'local') != 'local':
        parser.error('Only the synthetic local profile is implemented')
    if args.output.exists():
        parser.error('Output already exists; retain the original files and choose a new directory')
    now = datetime.now(timezone.utc)
    cutoff = datetime.fromisoformat(args.cutoff.replace('Z', '+00:00'))
    if cutoff.tzinfo is None or cutoff.timestamp() > now.timestamp() + 5:
        parser.error('Explicit valid provider cutoff timezone required')
    if not 1 <= (args.to_date - args.from_date).days <= 366 or args.to_date.toordinal() > now.date().toordinal() + 1:
        parser.error('Bounded half-open UTC reporting period required')
    with args.rows.open('rb') as stream:
        data = stream.read(2_000_001)
    if len(data) > 2_000_000:
        parser.error('Provider row input exceeds its 2 MB bound')
    rows = json.loads(data)
    if not isinstance(rows, list) or len(rows) > 1000 or any(not isinstance(row, dict) for row in rows):
        parser.error('Up to 1000 explicit provider row objects required')
    identity = 'paymentId' if args.kind == 'outgoing' else 'movementId'
    for row in rows:
        if identity not in row:
            parser.error('Provider row namespace does not match the selected kind')
        for field in ['principalMinor', 'providerFeeMinor']:
            value = row.get(field)
            if (not isinstance(value, str) or not re.fullmatch(r'0|-?[1-9][0-9]*', value)
                    or not -(2**63) <= int(value) < 2**63):
                parser.error('Amounts must remain canonical signed i64 decimal strings')
    period = {'fromDate': args.from_date.isoformat(), 'toDate': args.to_date.isoformat()}
    cutoff = cutoff.astimezone(timezone.utc).isoformat().replace('+00:00', 'Z')
    def new_file():
        return {'fileId': str(uuid.uuid4()), 'formatVersion': 1, 'provider': 'SYNTHETIC_RAIL_SETTLEMENT',
                'feePolicy': 'SYNTHETIC_PROVIDER_ZERO_FEE_V1', 'range': period, 'generatedAt': cutoff, 'rows': []}
    files = []
    current = new_file()
    for row in rows:
        trial = {**current, 'rows': current['rows'] + [row]}
        if len(trial['rows']) > 100 or len(encoded(trial)) > 60000:
            if not current['rows']:
                parser.error('One provider row exceeds a file byte bound')
            files.append(current)
            current = new_file()
            trial = {**current, 'rows': [row]}
            if len(encoded(trial)) > 60000:
                parser.error('One provider row exceeds a file byte bound')
        current = trial
    files.append(current)
    if len(files) > 10:
        parser.error('Provider snapshot needs more than ten bounded files; no files were written')
    manifest = {'setId': str(uuid.uuid4()), 'kind': 'OUTGOING' if args.kind == 'outgoing' else 'CREDITS',
                'range': period, 'generatedAt': cutoff, 'fileIds': [f['fileId'] for f in files]}
    # All source/bound validation precedes creation; never modify an existing dataset.
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    try:
        artifacts = {f'file-{n:03}.json': encoded(f) for n, f in enumerate(files, 1)}
        artifacts['settlement-set.json'] = encoded(manifest)
        for name, body in artifacts.items():
            descriptor = os.open(args.output / name, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
            with os.fdopen(descriptor, 'wb') as stream:
                stream.write(body)
    except BaseException:
        shutil.rmtree(args.output)
        raise
    print(f"Prepared {len(rows)} provider rows in {len(files)} files; set {manifest['setId']}. Nothing published.")


if __name__ == '__main__':
    main()
