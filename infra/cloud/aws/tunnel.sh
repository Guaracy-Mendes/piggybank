#!/bin/bash
set -euo pipefail
: "${AWS_REGION:?set AWS_REGION}"
: "${AWS_INSTANCE_ID:?set AWS_INSTANCE_ID from Terraform output}"
children=()
cleanup() { if ((${#children[@]})); then kill "${children[@]}" 2>/dev/null || true; wait "${children[@]}" 2>/dev/null || true; fi; }
trap cleanup EXIT
trap 'exit 130' INT TERM
ports=(4180 4181 4300 4301 18280)
if [[ ${BANK_TUNNEL_PROVIDER:-0} == 1 ]]; then ports+=(4182); fi
for port in "${ports[@]}"; do
  aws ssm start-session --region "$AWS_REGION" --target "$AWS_INSTANCE_ID" \
    --document-name AWS-StartPortForwardingSession \
    --parameters "{\"portNumber\":[\"$port\"],\"localPortNumber\":[\"$port\"]}" &
  children+=("$!")
done
# End the bundle when any tunnel exits; reconnect the complete bundle with this script.
wait -n "${children[@]}"
