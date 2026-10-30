#!/usr/bin/env bash
# Commit everything that changed in the working tree and push the task branch.
#   usage: autocommit.sh "<commit message>"       (run inside the repository)
# Prints `@@SYMPHONY {"type":"pushed",...}` for every push. Never forces: a rejected push is
# retried once after rebasing onto the remote branch, and fails otherwise.
set -uo pipefail

MESSAGE="${1:-Symphony cloud agent}"
BRANCH="${SYMPHONY_BRANCH:?}"

# A failed `git add` (an index lock, a permission problem) must not look like "nothing to commit".
git add -A || { echo "git add failed; nothing was committed" >&2; exit 1; }
if ! git diff --cached --quiet; then
  # Hooks never run (the repository is not trusted); the message is the only thing taken from the agent.
  git commit --quiet --no-verify -m "$MESSAGE" || exit 1
fi

HEAD_SHA="$(git rev-parse HEAD)"
REMOTE_SHA="$(git rev-parse --quiet --verify "refs/remotes/origin/${BRANCH}" || echo none)"
if [ "$HEAD_SHA" = "$REMOTE_SHA" ]; then
  exit 0
fi

push() { git push --quiet --no-verify origin "HEAD:refs/heads/${BRANCH}" 2>&1; }

if ! OUT="$(push)"; then
  # The remote moved (rare: only this sandbox pushes to the task branch). Rebase once, never force.
  if git fetch --quiet origin "+refs/heads/${BRANCH}:refs/remotes/origin/${BRANCH}" \
     && git rebase --quiet "origin/${BRANCH}" 2>/dev/null; then
    OUT="$(push)" || { echo "push failed: $OUT" >&2; exit 1; }
  else
    git rebase --abort 2>/dev/null || true
    echo "push rejected and the branch could not be rebased: $OUT" >&2
    exit 1
  fi
fi

SHA="$(git rev-parse HEAD)"
printf '@@SYMPHONY {"type":"pushed","sha":"%s","branch":"%s"}\n' "$SHA" "$BRANCH"
