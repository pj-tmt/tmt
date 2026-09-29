#!/bin/sh
# Runs a command up to <attempts> times. After a failed attempt it waits
# <delay> seconds, doubling the wait after each further failure, and it exits
# with the last status once the attempts are spent. For network steps at the
# start of CI jobs (Corepack, apt) that fail without any change to the
# repository; never wrap a step whose failure means the code is wrong.
#
# usage: retry-command.sh <attempts> <delay-seconds> <command> [argument...]
set -eu

usage() {
  printf '%s\n' 'usage: retry-command.sh <attempts> <delay-seconds> <command> [argument...]' >&2
  exit 2
}

[ "$#" -ge 3 ] || usage
attempts=$1
delay=$2
shift 2
case $attempts in '' | *[!0-9]* | 0) usage ;; esac
case $delay in '' | *[!0-9]*) usage ;; esac

attempt=1
while :; do
  status=0
  "$@" || status=$?
  [ "$status" -ne 0 ] || exit 0
  printf 'retry-command: %s failed on attempt %s of %s with status %s.\n' \
    "$1" "$attempt" "$attempts" "$status" >&2
  [ "$attempt" -lt "$attempts" ] || exit "$status"
  printf 'retry-command: retrying in %s s.\n' "$delay" >&2
  sleep "$delay"
  delay=$((delay * 2))
  attempt=$((attempt + 1))
done
