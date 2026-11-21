#!/usr/bin/env bash
# One-time setup for using the GitHub remote from this clone. Idempotent; changes only this clone's .git/config.
#  - origin -> https://github.com/Guaracy-Mendes/piggybank.git (override with PIGGYBANK_REMOTE)
#  - GitHub HTTPS credentials come from GITHUB_TOKEN in the git-ignored .env, then fall back to `gh auth`
#  - creates .env from .env.example (mode 0600) when it is missing
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
cd "$root"
remote="${PIGGYBANK_REMOTE:-https://github.com/Guaracy-Mendes/piggybank.git}"
if git remote get-url origin >/dev/null 2>&1; then
  git remote set-url origin "$remote"
else
  git remote add origin "$remote"
fi
# An empty value resets helpers inherited from global config; then .env first, gh as fallback.
git config --local --unset-all credential.https://github.com.helper 2>/dev/null || true
git config --local --add credential.https://github.com.helper ""
git config --local --add credential.https://github.com.helper '!"$(git rev-parse --show-toplevel)/scripts/git-credential-env.sh"'
if command -v gh >/dev/null 2>&1; then
  git config --local --add credential.https://github.com.helper '!gh auth git-credential'
fi
if [ ! -f .env ]; then
  (umask 077 && cp .env.example .env)
  echo "Created .env from .env.example; set GITHUB_TOKEN in it."
fi
chmod 600 .env
git check-ignore -q .env || { echo ".env is not git-ignored; refusing to continue." >&2; exit 1; }
git fetch --quiet origin || echo "Fetch failed: check GITHUB_TOKEN in .env (public reads normally work without it)." >&2
if git rev-parse --verify --quiet origin/main >/dev/null; then
  git branch --quiet --set-upstream-to=origin/main main 2>/dev/null || true
fi
echo "Remote: $(git remote get-url origin)"
echo "Credentials: .env GITHUB_TOKEN, then gh auth"
