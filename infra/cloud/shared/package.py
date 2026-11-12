#!/usr/bin/env python3
"""Build two Linux/amd64 images and a checksummed, secret-free cloud release."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[3]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('release', help='Immutable lowercase release identifier')
    parser.add_argument('--runtime', default=os.environ.get('CONTAINER_RUNTIME', 'docker'))
    args = parser.parse_args()
    if not re.fullmatch(r'[a-z0-9][a-z0-9.-]{0,62}', args.release):
        parser.error('Use 1-63 lowercase letters, digits, dots or hyphens')
    out = ROOT / '.local/cloud/releases'
    out.mkdir(parents=True, exist_ok=True)
    destination = out / f'{args.release}.tgz'
    if destination.exists():
        parser.error('Release already exists; choose a new immutable identifier')
    for target, name in [('runtime', 'runtime'), ('web', 'web')]:
        subprocess.run([args.runtime, 'build', '--platform', 'linux/amd64', '--target', target,
                        '-f', 'infra/cloud/shared/Dockerfile', '-t', f'piggybank-{name}:{args.release}', '.'],
                       cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(dir=out) as temporary:
        stage = Path(temporary)
        shutil.copytree(ROOT / 'infra/cloud/shared', stage / 'deployment',
                        ignore=shutil.ignore_patterns('__pycache__'))
        shutil.copytree(ROOT / 'infra/identity', stage / 'deployment/identity')
        shutil.copy2(ROOT / 'scripts/identity.py', stage / 'deployment/identity_provision.py')
        (stage / 'deployment/release.env').write_text(f'RELEASE_ID={args.release}\n')
        manifest = {'release': args.release, 'platform': 'linux/amd64', 'profile': 'synthetic-local',
                    'schemaVersion': max(int(p.name.split('_')[0]) for p in (ROOT / 'migrations').glob('*.sql')),
                    'files': {}}
        for path in sorted((stage / 'deployment').rglob('*')):
            if path.is_file():
                manifest['files'][str(path.relative_to(stage))] = hashlib.sha256(path.read_bytes()).hexdigest()
        images = [f'piggybank-runtime:{args.release}', f'piggybank-web:{args.release}']
        if args.runtime == 'podman':
            subprocess.run([args.runtime, 'save', '--format', 'docker-archive', '--multi-image-archive',
                            '-o', str(stage / 'images.tar'), *images], check=True)
        else:
            subprocess.run([args.runtime, 'save', '-o', str(stage / 'images.tar'), *images], check=True)
        manifest['imagesSha256'] = hashlib.file_digest((stage / 'images.tar').open('rb'), 'sha256').hexdigest()
        (stage / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        pending = stage / 'release.tgz'
        with tarfile.open(pending, 'w:gz') as archive:
            for name in ['deployment', 'images.tar', 'manifest.json']:
                archive.add(stage / name, arcname=name)
        pending.replace(destination)
    digest = hashlib.file_digest(destination.open('rb'), 'sha256').hexdigest()
    destination.with_suffix('.tgz.sha256').write_text(f'{digest}  {destination.name}\n')
    print(f'Release: {destination}\nSHA256: {digest}')

if __name__ == '__main__':
    main()
