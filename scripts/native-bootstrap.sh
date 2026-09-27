#!/bin/sh
# Generated from verified cargo-dist artifacts; do not maintain release facts here.
# HTTPS and embedded digests trust the official origin, not an independent signature.
set -eu

main() {
  version='@@VERSION@@'
  channel='@@CHANNEL@@'
  prefix=${HOME:?HOME is required}/.local
  pin=no
  skill=yes
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --prefix)
        [ "$#" -ge 2 ] || fail '--prefix requires an absolute directory.'
        prefix=$2
        shift 2 ;;
      --pin) pin=yes; shift ;;
      --no-skill) skill=no; shift ;;
      --help)
        printf '%s\n' 'Usage: sh installer.sh [--prefix /absolute/directory] [--pin] [--no-skill]'
        return 0 ;;
      *) fail "Unknown installer option: $1" ;;
    esac
  done
  case "$prefix" in /*) ;; *) fail '--prefix must be absolute.' ;; esac
  [ "$prefix" != / ] || fail 'Refusing the filesystem root as an installation prefix.'
  for utility in curl uname mktemp rm wc tar chmod; do
    command -v "$utility" >/dev/null 2>&1 || fail "Required utility is missing: $utility"
  done
  if command -v sha256sum >/dev/null 2>&1; then
    hash_tool=sha256sum
  elif command -v shasum >/dev/null 2>&1; then
    hash_tool=shasum
  else
    fail 'SHA-256 verification requires sha256sum or shasum.'
  fi
  host="$(uname -s):$(uname -m)"
  case "$host" in
@@TARGET_CASES@@
    *) fail "This installer has no verified artifact for $host." ;;
  esac

  umask 077
  # Only successful mktemp creation grants deletion ownership. The trap is
  # installed immediately; no existing install or abandoned stage is removed.
  stage=$(mktemp -d "${TMPDIR:-/tmp}/tmt-bootstrap.XXXXXXXX")
  trap 'cleanup' 0
  trap 'exit 130' INT
  trap 'exit 143' TERM HUP
  base="https://github.com/wkh237/tmt/releases/download/v$version"
  download "$base/dist-manifest.json" "$stage/dist-manifest.json" @@MANIFEST_SIZE@@
  verify "$stage/dist-manifest.json" @@MANIFEST_SIZE@@ '@@MANIFEST_HASH@@'
  download "$base/$archive" "$stage/$archive" "$archive_size"
  verify "$stage/$archive" "$archive_size" "$archive_hash"

  # Do not extract archive paths into the filesystem. The generator has checked
  # the complete archive inventory; its digest binds this one selected stream.
  tar -xzOf "$stage/$archive" "${archive%.tar.gz}/tmt" > "$stage/tmt"
  chmod 700 "$stage/tmt"
  set -- __native-install --archive "$stage/$archive" \
    --manifest "$stage/dist-manifest.json" --prefix "$prefix" --channel "$channel"
  if [ "$pin" = yes ]; then set -- "$@" --pin; fi
  if "$stage/tmt" "$@"; then
    :
  else
    status=$?
    printf '%s\n' 'Native installation did not finish cleanly; inspect the error above before retrying.' \
      'If the prefix contains an npm/pnpm/brew/manual command, choose another prefix or remove it with its original manager. This installer never overwrites that command.' >&2
    return "$status"
  fi

  printf '%s\n' "Native tmt is installed at $prefix/bin/tmt."
  selected=$(command -v tmt || :)
  selected_alias=$(command -v tmux-team || :)
  if [ "$selected" != "$prefix/bin/tmt" ] || [ "$selected_alias" != "$prefix/bin/tmux-team" ]; then
    printf '%s\n' "PATH may still select another installation (tmt: ${selected:-not found}; tmux-team: ${selected_alias:-not found})." \
      "Put $prefix/bin first in PATH, then run hash -r or open a new shell." \
      'If replacing an npm/pnpm installation, remove it with npm uninstall -g tmux-team or pnpm remove -g tmux-team using its original manager.' \
      'For Homebrew/manual installs, use their original uninstall procedure. No old installation or shell profile was changed.'
  fi
  if [ "$skill" = yes ]; then
    if ! "$prefix/bin/tmt" install; then
      printf '%s\n' 'The native binary is installed, but skill setup failed. Inspect conflicts before explicitly running the new tmt install --force; it backs up conflicting links/content.' \
        'No data was migrated or deleted. Retry with the absolute new tmt path.' >&2
      return 1
    fi
    printf '%s\n' 'Reload your agent, or read tmt learn --skill in an existing conversation.'
  fi
  printf '%s\n' 'Use the new tmt upgrade for subsequent native updates. Application data was not migrated or deleted.'
}

fail() { printf '%s\n' "$*" >&2; exit 1; }

download() {
  # Disable user curl configuration; never accept an inherited insecure flag.
  curl --disable --fail --silent --show-error --location --proto '=https' \
    --proto-redir '=https' --max-redirs 3 --connect-timeout 10 --max-time 60 \
    --max-filesize "$3" --output "$2" "$1"
}

verify() {
  actual_size=$(wc -c < "$1")
  [ "$actual_size" -eq "$2" ] || fail 'Downloaded asset size mismatch.'
  if [ "$hash_tool" = sha256sum ]; then
    actual_hash=$(sha256sum "$1")
  else
    actual_hash=$(shasum -a 256 "$1")
  fi
  actual_hash=${actual_hash%% *}
  [ "$actual_hash" = "$3" ] || fail 'Downloaded asset SHA-256 mismatch.'
}

cleanup() {
  status=$?
  trap - 0
  if ! rm -rf -- "$stage"; then
    printf '%s\n' 'Could not remove the private bootstrap staging directory.' >&2
    [ "$status" -ne 0 ] || status=1
  fi
  exit "$status"
}

# Parse the complete final compound command before invoking main. A download
# truncated after the word 'main' must not run with missing caller arguments.
{
  main "$@"
}
