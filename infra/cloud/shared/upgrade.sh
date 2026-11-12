#!/bin/bash
set -euo pipefail
umask 077
provider=${1:?gcp or aws}
bucket=${2:?artifact bucket}
object=${3:?immutable releases/name.tgz object}
digest=${4:?expected SHA256}
[[ "$object" =~ ^releases/[a-z0-9][a-z0-9.-]*\.tgz$ && "$digest" =~ ^[a-f0-9]{64}$ ]] || exit 2
archive="/opt/piggybank-bootstrap/$digest.tgz"
python3 /srv/piggybank/current/deployment/transfer.py "$provider" download "$archive" "$bucket" "$object"
echo "$digest  $archive" | sha256sum --check --status
mkdir -p "/opt/piggybank-bootstrap/$digest"
tar -xzf "$archive" -C "/opt/piggybank-bootstrap/$digest" deployment/install-release.sh
bash "/opt/piggybank-bootstrap/$digest/deployment/install-release.sh" "$archive" "$digest"
