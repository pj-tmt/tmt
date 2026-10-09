#!/bin/bash
# Only the first dependency installs in the two fresh release-proof checkouts.
set -eu
CLI_PACKAGE='tmux-team'
RETRY_NOTE='pnpm install retry after Node async-hook abort (#1806)'

case $#:$* in
  0:) set -- install --frozen-lockfile --ignore-scripts ;;
  1:--trusted-cli) set -- --filter "$CLI_PACKAGE" --fail-if-no-match install --frozen-lockfile --ignore-scripts ;;
  *) echo 'unsupported native verification install mode' >&2; exit 2 ;;
esac

refuse() { echo "native verification install refused: $1" >&2; exit 2; }
workspace=${GITHUB_WORKSPACE:-}
[ -n "$workspace" ] || refuse 'missing GITHUB_WORKSPACE'
case $PWD in
  "$workspace/typescript"|"$workspace/release-source/typescript") ;;
  *) refuse 'unexpected checkout/cwd' ;;
esac
[ "$PWD" = "$(pwd -P)" ] && [ "$workspace" = "$(cd "$workspace" && pwd -P)" ] || refuse 'symlinked checkout/cwd'
root=${PWD%/typescript}
[ "$(git rev-parse --show-toplevel)" = "$root" ] || refuse 'unexpected git checkout root'
for file in package.json pnpm-lock.yaml pnpm-workspace.yaml; do
  [ -f "$file" ] && [ ! -L "$file" ] || refuse "missing or symlinked $file"
done

# Do not traverse Git or the separately owned candidate checkout. No symlink following.
modules() {
  find "$root" \( -path "$root/.git" -o -path "$root/release-source" \) -prune -o -name node_modules -prune "$@"
}
before=$(modules -print) || refuse 'cannot inspect modules'
[ -z "$before" ] || refuse 'pre-existing node_modules'
outputs=$(mktemp -d "${RUNNER_TEMP:?}/native-verification-install.XXXXXX")

summary() {
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    printf '%s; attempts=%s; final status=%s; logs=%s\n' "$1" "$attempt" "$status" "$outputs" >> "$GITHUB_STEP_SUMMARY" || echo 'could not append install summary' >&2
  fi
}

attempt=1
while :; do
  status=0
  pnpm "$@" > "$outputs/attempt-$attempt.stdout" 2> "$outputs/attempt-$attempt.stderr" || status=$?
  cat "$outputs/attempt-$attempt.stdout" || :
  cat "$outputs/attempt-$attempt.stderr" >&2 || :
  note='pnpm native verification install (#1806)'
  [ "$attempt" -eq 1 ] || note=$RETRY_NOTE
  if [ "$status" -eq 0 ] || [ "$attempt" -eq 2 ] || ! grep -Eq '^Error: async hook stack has become corrupted \(actual: [0-9]+, expected: [0-9]+\)$' "$outputs/attempt-1.stderr"; then
    summary "$note"
    exit "$status"
  fi
  printf '::warning::%s; attempt 1 failed with status %s; one extra attempt in 5 s.\n' "$RETRY_NOTE" "$status" >&2
  # Admission proved these modules absent: only this install could have created them.
  if ! { modules -exec rm -rf -- '{}' + && remaining=$(modules -print) && [ -z "$remaining" ]; }; then
    echo 'partial modules cleanup failed; no second attempt' >&2
    summary "$RETRY_NOTE; cleanup failed"
    exit "$status"
  fi
  sleep 5 || { summary "$RETRY_NOTE; wait failed"; exit "$status"; }
  attempt=2
done
