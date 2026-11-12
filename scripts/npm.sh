#!/usr/bin/env bash
set -euo pipefail
task_root="$(cd "$(dirname "$0")/.." && pwd)"
node_version="$(cat "$task_root/.node-version")"
node_path="${NVM_DIR:-$HOME/.nvm}/versions/node/v$node_version/bin"
if [ -x "$node_path/node" ]; then
  export PATH="$node_path:$PATH"
fi
if [ "$(node --version)" != "v$node_version" ]; then
  printf 'Node %s is required; select it with your Node version manager.\n' "$node_version" >&2
  exit 1
fi
cd "$task_root"
exec npm "$@"
