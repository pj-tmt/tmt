#!/bin/sh
# Runs one tmux command against a private scratch server, never the host's.
#
# usage: scripts/private-tmux.sh <scratch-dir> [-u|-2] <tmux command> [args...]
#
# An agent shell inherits TMUX and TMUX_PANE from the pane it runs in, so a bare
# `tmux` (kill-server above all) reaches the host's default server. This wrapper
# is the one way to start, drive and stop a scratch server from such a shell:
# - the socket is always <scratch-dir>/tmux.sock, passed with an explicit -S;
# - TMUX and TMUX_PANE are removed from the command's environment;
# - the scratch directory is refused, before anything is created, when it lies
#   inside a host tmux socket directory (/tmp/tmux-<uid>, /private/tmp/tmux-<uid>,
#   $TMUX_TMPDIR/tmux-<uid>, or the directory of the socket $TMUX names);
# - a symlinked socket, a relative directory and any tmux global option other
#   than -u and -2 (-S, -L, -f, -c, -T would choose another server) are refused.
# Exit status 2 means refused; any other status is tmux's own.
set -eu

refuse() {
  printf 'private-tmux: %s\n' "$1" >&2
  exit 2
}

# The physical form of $1, even when its tail does not exist yet.
resolve() {
  path=$1
  tail=
  while [ ! -d "$path" ]; do
    tail=/$(basename -- "$path")$tail
    path=$(dirname -- "$path")
  done
  printf '%s%s\n' "$(CDPATH='' cd -P -- "$path" && pwd -P)" "$tail"
}

[ $# -ge 2 ] || refuse 'usage: private-tmux.sh <scratch-dir> [-u|-2] <tmux command> [args...]'
dir=$1
shift
case $dir in
  /*) ;;
  *) refuse "the scratch directory must be an absolute path: $dir" ;;
esac
# mkdir -p would create a missing component before following a later `..`, so a
# path that climbs back out is judged by what it names, not by where it ends.
case /$dir/ in
  */../*) refuse "the scratch directory must not contain a .. component: $dir" ;;
esac
physical=$(resolve "$dir")

uid=$(id -u)
forbidden="/tmp/tmux-$uid /private/tmp/tmux-$uid"
if [ -n "${TMUX_TMPDIR:-}" ]; then
  forbidden="$forbidden ${TMUX_TMPDIR%/}/tmux-$uid"
fi
if [ -n "${TMUX:-}" ]; then
  forbidden="$forbidden $(dirname -- "${TMUX%%,*}")"
fi
for host in $forbidden; do
  case $host in
    /*) ;;
    *) continue ;;
  esac
  host=$(resolve "$host")
  case "$physical/" in
    "$host"/*) refuse "$dir lies inside the host tmux socket directory $host" ;;
  esac
done

flags=
while [ $# -gt 0 ]; do
  case $1 in
    -u | -2)
      flags="$flags $1"
      shift
      ;;
    -*) refuse "tmux option $1 is not allowed before the command; the socket is fixed" ;;
    *) break ;;
  esac
done
[ $# -gt 0 ] || refuse 'no tmux command given'

# tmux binds the path as given (the kernel limits that string to 104 bytes on
# macOS); the checks above judged its physical form.
socket=${dir%/}/tmux.sock
[ ! -L "$socket" ] || refuse "$socket is a symlink"
[ "${#socket}" -le 103 ] || refuse "$socket is too long for a Unix socket path"

# Only a directory this script creates is tightened; an existing one is left alone.
if [ ! -d "$physical" ]; then
  mkdir -p -- "$physical"
  chmod 700 "$physical"
fi
# shellcheck disable=SC2086 # $flags holds only -u and -2
exec env -u TMUX -u TMUX_PANE TMUX_TMPDIR="$physical" tmux $flags -f /dev/null -S "$socket" "$@"
