#!/bin/sh
# Run the complete verifier, including its shell installers and child binaries,
# with the target's macOS execution preference. The caller supplies Bash on stdin.
set -eu
if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  echo 'Usage: run-native-verification.sh <target> < verification.bash' >&2
  exit 2
fi
if [ "$1" = x86_64-apple-darwin ]; then
  verification=$(mktemp "${TMPDIR:-/tmp}/tmt-native-verification.XXXXXX")
  trap 'rm -f "$verification"' 0
  cat > "$verification" <<'HOST'
test "$(/usr/bin/uname -m)" = x86_64
test "$(node -p process.arch)" = x64
HOST
  cat >> "$verification"
  /usr/bin/arch -x86_64 /bin/bash --noprofile --norc -euo pipefail "$verification"
  exit
fi
exec /bin/bash --noprofile --norc -euo pipefail
