#!/usr/bin/env bash
# Entry point the bridge runs for one agent turn: put the task branch in the sandbox, then hand
# over to the driver of the chosen agent. Credentials are not handled here: the bridge connected
# with a git session, so `git` already authenticates to GitHub.
set -euo pipefail

: "${SYMPHONY_REPO_URL:?}" "${SYMPHONY_BRANCH:?}" "${SYMPHONY_REPO_DIR:?}"
DRIVERS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

git config --global user.name "${SYMPHONY_GIT_NAME:-Symphony Cloud Agent}"
git config --global user.email "${SYMPHONY_GIT_EMAIL:-symphony-cloud-agent@users.noreply.github.com}"
git config --global --add safe.directory "$SYMPHONY_REPO_DIR"
git config --global core.hooksPath /dev/null

if [ ! -d "$SYMPHONY_REPO_DIR/.git" ]; then
  mkdir -p "$(dirname "$SYMPHONY_REPO_DIR")"
  git clone --quiet "$SYMPHONY_REPO_URL" "$SYMPHONY_REPO_DIR"
fi
cd "$SYMPHONY_REPO_DIR"
git remote set-url origin "$SYMPHONY_REPO_URL"

# Fetch the branch the local worktree published. A resumed sandbox keeps its own commits: they are
# only ever fast-forwarded, never reset.
git fetch --quiet origin "+refs/heads/${SYMPHONY_BRANCH}:refs/remotes/origin/${SYMPHONY_BRANCH}"
if git show-ref --verify --quiet "refs/heads/${SYMPHONY_BRANCH}"; then
  git checkout --quiet "$SYMPHONY_BRANCH"
  git merge --ff-only --quiet "origin/${SYMPHONY_BRANCH}" || true
else
  git checkout --quiet -b "$SYMPHONY_BRANCH" "origin/${SYMPHONY_BRANCH}"
fi

exec node "$DRIVERS/symphony-run-agent.ts"
