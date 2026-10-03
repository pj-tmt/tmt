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
if ! host_arch=$(/usr/bin/uname -m); then
  echo 'Intel verification host check failed: uname -m could not run.' >&2
  exit 1
fi
if [ "$host_arch" != x86_64 ]; then
  printf 'Intel verification host check failed: uname -m expected x86_64, got %s.\n' "$host_arch" >&2
  exit 1
fi
if ! node_arch=$(node -p process.arch); then
  echo 'Intel verification host check failed: node -p process.arch could not run.' >&2
  exit 1
fi
if [ "$node_arch" != x64 ]; then
  printf 'Intel verification host check failed: Node process.arch expected x64, got %s (node: %s).\n' "$node_arch" "$(command -v node)" >&2
  exit 1
fi
HOST
  cat >> "$verification"
  /usr/bin/arch -x86_64 /bin/bash --noprofile --norc -euo pipefail "$verification"
  exit
fi
exec /bin/bash --noprofile --norc -euo pipefail
