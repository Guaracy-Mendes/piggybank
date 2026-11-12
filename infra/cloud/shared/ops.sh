#!/bin/bash
set -euo pipefail
umask 077
base=/srv/piggybank
cd "$base/current/deployment"
compose() { docker compose --env-file "$base/state/.env" --env-file release.env -f compose.yaml "$@"; }
case "${1:-status}" in
  start)
    mountpoint -q "$base" || { echo 'Persistent disk is not mounted'; exit 1; }
    compose up -d postgres nats
    python3 wait.py http://127.0.0.1:18222/healthz
    compose up -d keycloak
    python3 wait.py http://127.0.0.1:18280/realms/customers/.well-known/openid-configuration
    compose up -d provider api worker web
    python3 smoke.py
    ;;
  stop) compose stop ;;
  status) compose ps; python3 smoke.py ;;
  logs) compose logs --tail 100 "${2:-api}" ;;
  backup)
    if [[ "${BANK_CLOUD_LOCK_HELD:-0}" != 1 ]]; then
      exec 9>/run/piggybank-release.lock
      flock -n 9 || { echo 'Another backup, deployment or restore is active'; exit 1; }
      export BANK_CLOUD_LOCK_HELD=1
    fi
    mkdir -p "$base/backups"
    stamp=$(date -u +%Y%m%dT%H%M%SZ)
    target="$base/backups/$stamp.tgz"
    compose stop
    # Include PG, Keycloak, NATS, evidence, keys and fixture credentials at one stopped cut.
    trap 'compose up -d postgres nats keycloak provider api worker web' EXIT
    sync
    tar --numeric-owner --acls --xattrs -czf "$target" -C "$base" state
    sha256sum "$target" > "$target.sha256"
    cp "$base/current/manifest.json" "$target.manifest.json"
    readlink -f "$base/current" > "$target.release-path"
    echo "Consistent stopped backup: $target (contains credentials; encrypt/restrict off-host storage)"
    ;;
  *) echo 'Usage: piggybank {start|stop|status|logs [service]|backup}'; exit 2 ;;
esac
