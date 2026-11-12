#!/usr/bin/env bash
set -euo pipefail
task_root="$(cd "$(dirname "$0")/.." && pwd)"
toolchain_version="$(python3 -c 'import tomllib,sys; print(tomllib.load(open(sys.argv[1],"rb"))["toolchain"]["channel"])' "$task_root/rust-toolchain.toml")"
toolchain_path="${RUSTUP_HOME:-$HOME/.rustup}/toolchains/${toolchain_version}-$(uname -m)-unknown-linux-gnu/bin"
if [ -x "$toolchain_path/cargo" ]; then
  export PATH="$toolchain_path:$PATH"
elif command -v rustup >/dev/null 2>&1; then
  toolchain_path="$(rustup which --toolchain "$toolchain_version" cargo)"
  export PATH="$(dirname "$toolchain_path"):$PATH"
elif [ -x "$HOME/.cargo/bin/rustup" ]; then
  toolchain_path="$("$HOME/.cargo/bin/rustup" which --toolchain "$toolchain_version" cargo)"
  export PATH="$(dirname "$toolchain_path"):$PATH"
else
  printf 'Install Rust %s with rustup: https://rustup.rs\n' "$toolchain_version" >&2
  exit 1
fi
cd "$task_root"
exec cargo "$@"
