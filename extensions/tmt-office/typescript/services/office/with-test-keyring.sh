#!/bin/sh
# Container-only real Secret Service fixture; never select a host credential store.
set -eu
test -e /.dockerenv
test "${GCLOUD_PROJECT:-}" = demo-tmt-office
if test "${1:-}" != --session; then
  exec dbus-run-session -- sh "$0" --session "$@"
fi
shift
vault_dir=$(mktemp -d /tmp/tmt-office-test-keyring.XXXXXXXX)
mkdir "$vault_dir/home" "$vault_dir/runtime" "$vault_dir/control"
chmod 700 "$vault_dir" "$vault_dir/home" "$vault_dir/runtime" "$vault_dir/control"
export XDG_RUNTIME_DIR="$vault_dir/runtime"
export GNOME_KEYRING_CONTROL="$vault_dir/control"
printf '%s' 'isolated-test-keyring-password' |
  HOME="$vault_dir/home" gnome-keyring-daemon --foreground --unlock --components=secrets --control-directory="$GNOME_KEYRING_CONTROL" > "$vault_dir/environment" 2> "$vault_dir/diagnostics" &
vault_pid=$!
cleanup() {
  kill "$vault_pid" 2>/dev/null || true
  wait "$vault_pid" 2>/dev/null || true
  case "$vault_dir" in /tmp/tmt-office-test-keyring.*) rm -rf -- "$vault_dir" ;; *) exit 1 ;; esac
}
trap cleanup EXIT
gdbus wait --session --timeout=10 org.freedesktop.secrets
"$@"
