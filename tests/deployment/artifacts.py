"""Verify and extract the actual immutable release used by local image acceptance."""
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import tarfile


def checksum_file(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def extract_release(archive: Path, destination: Path, release: str) -> dict:
    if not re.fullmatch(r'[a-z0-9][a-z0-9.-]{0,62}', release):
        raise ValueError('Invalid immutable release identifier')
    checksum = archive.with_suffix('.tgz.sha256').read_text().split()
    if len(checksum) != 2 or checksum[1] != archive.name:
        raise ValueError('Release checksum identity mismatch')
    if checksum_file(archive) != checksum[0]:
        raise ValueError('Release archive checksum mismatch')
    with tarfile.open(archive, 'r:gz') as source:
        members = source.getmembers()
        names = set()
        for member in members:
            path = PurePosixPath(member.name)
            if (not path.parts or path.is_absolute() or '..' in path.parts or str(path) != member.name
                    or member.name in names or not (member.isfile() or member.isdir())
                    or (path.parts[0] != 'deployment' and member.name not in ['images.tar', 'manifest.json'])):
                raise ValueError('Unsafe release archive membership')
            names.add(member.name)
        manifest = json.load(source.extractfile('manifest.json'))
        if (manifest['release'] != release or manifest['platform'] != 'linux/amd64'
                or manifest['profile'] != 'synthetic-local' or not isinstance(manifest['schemaVersion'], int)):
            raise ValueError('Release manifest identity mismatch')
        expected = set(manifest['files']) | {'images.tar', 'manifest.json'}
        if {m.name for m in members if m.isfile()} != expected:
            raise ValueError('Release manifest membership mismatch')
        for member in members:
            target = destination / member.name
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with source.extractfile(member) as incoming, target.open('xb') as outgoing:
                shutil.copyfileobj(incoming, outgoing)
        for name, digest in {**manifest['files'], 'images.tar': manifest['imagesSha256']}.items():
            if checksum_file(destination / name) != digest:
                raise ValueError('Release member checksum mismatch')
    return manifest
