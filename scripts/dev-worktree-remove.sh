#!/bin/sh
# Removes a finished linked worktree only when nothing can be lost: it must be
# clean, and either its PR is merged or its commits are all on its upstream.
# A refusal is final: stop and ask the maintainer; never remove it by hand or
# with --force. `git log @{u}..` alone is not the test, because the maintainer
# can update a PR branch on the server (a rebase), which leaves the local
# commits "unpushed" although the PR carries them.
# Usage: scripts/dev-worktree-remove.sh <worktree-path> <pr-number>
set -eu

if [ "$#" -ne 2 ]; then
  printf '%s\n' 'Usage: scripts/dev-worktree-remove.sh <worktree-path> <pr-number>' >&2
  exit 2
fi
path=$1
pr=$2
case $pr in
  '' | *[!0-9]*)
    printf '%s\n' 'The PR number must be digits only.' >&2
    exit 2
    ;;
esac

refuse() {
  printf 'Not removing %s: %s\nStop and ask the maintainer.\n' "$path" "$1" >&2
  exit 1
}

git -C "$path" rev-parse --git-dir > /dev/null 2>&1 || refuse 'it is not a git worktree.'
git_dir=$(git -C "$path" rev-parse --path-format=absolute --git-dir)
common_dir=$(git -C "$path" rev-parse --path-format=absolute --git-common-dir)
[ "$git_dir" != "$common_dir" ] || refuse 'it is the main checkout, not a linked worktree.'

[ -z "$(git -C "$path" status --short)" ] || refuse 'it has uncommitted or untracked changes.'

state=$(gh pr view "$pr" --json state -q .state 2> /dev/null || true)
if [ "$state" != MERGED ]; then
  git -C "$path" rev-parse --verify -q '@{u}' > /dev/null \
    || refuse "PR $pr is not merged (state: ${state:-unknown}) and the branch has no upstream."
  [ -z "$(git -C "$path" log --oneline '@{u}..')" ] \
    || refuse "PR $pr is not merged (state: ${state:-unknown}) and the branch has commits its upstream lacks."
fi

git -C "$common_dir/.." worktree remove "$path"
git -C "$common_dir/.." worktree prune
printf 'Removed %s\n' "$path"
