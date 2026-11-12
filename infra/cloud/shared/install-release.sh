#!/bin/bash
# Run as root after mounting /srv/piggybank and downloading the release archive.
set -euo pipefail
umask 077
archive=${1:?release archive path required}
digest=${2:?expected SHA256 required}
[[ "$digest" =~ ^[a-f0-9]{64}$ ]] || exit 2
base=/srv/piggybank
mountpoint -q "$base" || { echo 'Persistent disk missing'; exit 1; }
if [[ "${BANK_CLOUD_LOCK_HELD:-0}" != 1 ]]; then
  exec 9>/run/piggybank-release.lock
  flock -n 9 || { echo 'Another backup, deployment or restore is active'; exit 1; }
  export BANK_CLOUD_LOCK_HELD=1
fi
echo "$digest  $archive" | sha256sum --check --status
mkdir -p "$base/releases"
destination="$base/releases/$digest"
if [[ ! -d "$destination" ]]; then
  mkdir "$destination"
  tar --extract --gzip --file "$archive" --directory "$destination" --no-same-owner
fi
python3 - "$destination" <<'PY'
import hashlib, json, pathlib, re, sys
root = pathlib.Path(sys.argv[1])
m = json.loads((root/'manifest.json').read_text())
assert re.fullmatch(r'[a-z0-9][a-z0-9.-]{0,62}', m['release'])
assert m['platform']=='linux/amd64' and m['profile']=='synthetic-local'
for name, digest in m['files'].items():
    path = (root/name).resolve()
    assert path.is_relative_to(root.resolve())
    assert hashlib.file_digest(path.open('rb'), 'sha256').hexdigest()==digest, name
assert hashlib.file_digest((root/'images.tar').open('rb'), 'sha256').hexdigest()==m['imagesSha256']
PY
if [[ -L "$base/current" && "$(readlink -f "$base/current")" != "$destination" ]]; then
  if [[ "${BANK_CLOUD_RESTORING:-0}" != 1 ]]; then /usr/local/bin/piggybank backup; fi
  /usr/local/bin/piggybank stop
  readlink -f "$base/current" > "$base/previous-release"
fi
docker load --input "$destination/images.tar" > /dev/null
ln -sfn "$destination" "$base/current.next"
mv -Tf "$base/current.next" "$base/current"
cd "$base/current/deployment"
python3 bootstrap.py environment
mkdir -p "$base/state/evidence" "$base/state/nats"
chown 10001:10001 "$base/state/evidence"
compose() { docker compose --env-file "$base/state/.env" --env-file release.env -f compose.yaml "$@"; }
compose up -d --wait postgres
python3 bootstrap.py database
compose run --rm -T migrate
compose up -d nats keycloak provider
python3 wait.py http://127.0.0.1:18222/healthz
python3 wait.py http://127.0.0.1:18280/realms/customers/.well-known/openid-configuration
if [[ ! -f "$base/state/.initialized" ]]; then
  python3 bootstrap.py seed
  touch "$base/state/.initialized"
fi
bash activate-host.sh
compose up -d provider api worker web
python3 wait.py http://127.0.0.1:4180/health/ready
python3 wait.py http://127.0.0.1:4181/health/ready
python3 smoke.py
systemctl start piggybank
printf '%s\n' "$digest" > "$base/state/deployed-release-sha256"
echo 'Release installed; synthetic cloud smoke passed.'
