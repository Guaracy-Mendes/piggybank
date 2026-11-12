#!/bin/bash
set -euo pipefail
: "${GCP_PROJECT:?set GCP_PROJECT}"
zone=${GCP_ZONE:-us-central1-a}
instance=${GCP_INSTANCE:-piggybank-eval}
# All ports retain their original numbers for BFF/issuer/cookie origins.
provider_forward=()
if [[ ${BANK_TUNNEL_PROVIDER:-0} == 1 ]]; then
  provider_forward=(-L 127.0.0.1:4182:127.0.0.1:4182)
fi
exec gcloud compute ssh "$instance" --project "$GCP_PROJECT" --zone "$zone" --tunnel-through-iap -- \
  "${provider_forward[@]}" -N -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 \
  -L 127.0.0.1:4180:127.0.0.1:4180 -L 127.0.0.1:4181:127.0.0.1:4181 \
  -L 127.0.0.1:4300:127.0.0.1:4300 -L 127.0.0.1:4301:127.0.0.1:4301 \
  -L 127.0.0.1:18280:127.0.0.1:18280
