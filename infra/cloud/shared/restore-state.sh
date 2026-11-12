#!/bin/bash
# Explicit maintenance restore: original PG+NATS+evidence+keys together, matching release.
set -euo pipefail
umask 077
backup=${1:?backup .tgz path}
backup_sha=${2:?backup SHA256}
archive=${3:?matching release .tgz path}
release_sha=${4:?matching release SHA256}
[[ "$backup_sha" =~ ^[a-f0-9]{64}$ && "$release_sha" =~ ^[a-f0-9]{64}$ ]] || exit 2
base=/srv/piggybank
mountpoint -q "$base" || exit 1
if [[ "${BANK_CLOUD_LOCK_HELD:-0}" != 1 ]]; then
  exec 9>/run/piggybank-release.lock
  flock -n 9 || { echo 'Another backup, deployment or restore is active'; exit 1; }
  export BANK_CLOUD_LOCK_HELD=1
fi
echo "$backup_sha  $backup" | sha256sum --check --status
echo "$release_sha  $archive" | sha256sum --check --status
python3 - "$backup" "$archive" <<'PY'
import json, sys, tarfile
from pathlib import PurePosixPath
backup,release=sys.argv[1:]
expected=json.load(open(backup+'.manifest.json'))
with tarfile.open(release) as archive:
    actual=json.load(archive.extractfile('manifest.json'))
assert actual==expected, 'Backup requires its exact original release manifest'
with tarfile.open(backup) as archive:
    for member in archive.getmembers():
        path=PurePosixPath(member.name)
        assert not path.is_absolute() and '..' not in path.parts and path.parts[0]=='state'
        assert not member.issym() and not member.islnk(), 'Unexpected backup link'
PY
systemctl stop piggybank
/usr/local/bin/piggybank stop
# Preserve the failed state for diagnosis; never merge old and restored directories.
mv "$base/state" "$base/state.before-restore-$(date -u +%Y%m%dT%H%M%SZ)"
tar --numeric-owner --acls --xattrs -xzf "$backup" -C "$base"
# Reinstall only after the original state/credentials have been restored.
BANK_CLOUD_RESTORING=1 bash /srv/piggybank/current/deployment/install-release.sh "$archive" "$release_sha"
echo 'Restore complete. Reconcile uncertain work by original identity before resuming customer activity.'
