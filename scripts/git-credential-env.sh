#!/usr/bin/env bash
# Git credential helper: answers GitHub HTTPS requests with GITHUB_TOKEN from the repository's git-ignored .env.
# The file is read as data (never sourced). With no token it prints nothing, so git falls through to the next helper.
# Enable once per clone:  scripts/setup_git_remote.sh
set -euo pipefail
[ "${1:-}" = "get" ] || exit 0
host=""
while IFS= read -r line && [ -n "$line" ]; do
  case "$line" in host=*) host="${line#host=}" ;; esac
done
[ "$host" = "github.com" ] || exit 0
env_file="$(git rev-parse --show-toplevel)/.env"
[ -f "$env_file" ] || exit 0
token="$(grep -E '^GITHUB_TOKEN=' "$env_file" | tail -n 1 | cut -d= -f2- | tr -d "\"' \r")"
[ -n "$token" ] || exit 0
printf 'username=x-access-token\npassword=%s\n' "$token"
