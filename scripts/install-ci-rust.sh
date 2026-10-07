#!/bin/sh
# Only disposable Actions jobs may remove runner-image toolchains. Keep install
# arguments at their workflow owner; cache keys must see just that job's pin.
set -eu

fail() {
  printf 'install-ci-rust: %s\n' "$1" >&2
  exit 1
}

[ "${GITHUB_ACTIONS:-}" = true ] || fail 'requires a disposable GitHub Actions job'
[ "$#" -ge 1 ] || fail 'requires the pinned toolchain and installation arguments'
toolchain=$1
shift
case "$toolchain" in
  '' | *[!0-9.]* | .* | *..* | *.) fail 'requires a numeric pinned toolchain' ;;
esac

rustup toolchain install "$toolchain" "$@"
rustup default "$toolchain"
default=$(rustup default)
retained=${default%% *}
case "$retained" in
  "$toolchain"-*) ;;
  *) fail 'default does not resolve to the requested host toolchain' ;;
esac
active=$(rustup show active-toolchain)
[ "${active%% *}" = "$retained" ] || fail 'an override selects a different toolchain'

before=$(rustup toolchain list --quiet)
printf 'Installed toolchains before normalization:\n%s\n' "$before"
while read -r installed annotation; do
  [ -n "$installed" ] || fail 'empty installed toolchain record'
  if [ "$installed" != "$retained" ]; then
    rustup toolchain uninstall "$installed"
  fi
done <<EOF_TOOLCHAINS
$before
EOF_TOOLCHAINS

after=$(rustup toolchain list --quiet)
printf 'Installed toolchains after normalization:\n%s\n' "$after"
count=0
while read -r installed annotation; do
  [ "$installed" = "$retained" ] || fail 'installed toolchain set is not the requested singleton'
  count=$((count + 1))
done <<EOF_TOOLCHAINS
$after
EOF_TOOLCHAINS
[ "$count" -eq 1 ] || fail 'installed toolchain set is not a singleton'
active=$(rustup show active-toolchain)
[ "${active%% *}" = "$retained" ] || fail 'selected toolchain changed during normalization'
rustup run "$toolchain" rustc -vV
