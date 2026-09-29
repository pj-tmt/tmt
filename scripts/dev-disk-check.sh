#!/bin/sh
# Read-only disk check before a Docker suite: free space where the checkout
# lives, then Docker's own usage report. It deletes nothing and never fails on
# a full disk; the warning is for the person about to start a large build.
# TMT_DISK_WARN_GB (default 30) is the only threshold, kept here once.
set -eu

threshold=${TMT_DISK_WARN_GB:-30}
case $threshold in
  '' | *[!0-9]*)
    printf '%s\n' 'TMT_DISK_WARN_GB must be a whole number of gigabytes.' >&2
    exit 2
    ;;
esac

repository=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)

# POSIX df: the fourth column of the data row is available 1024-byte blocks.
free_kb=$(df -Pk "$repository" 2>/dev/null | awk 'NR == 2 { print $4 }')
case $free_kb in
  '' | *[!0-9]*)
    printf '%s\n' "Could not read free disk space for $repository."
    ;;
  *)
    free_gb=$((free_kb / 1048576))
    printf 'Free disk space at %s: %s GB\n' "$repository" "$free_gb"
    if [ "$free_gb" -lt "$threshold" ]; then
      printf '%s\n' \
        "WARNING: less than $threshold GB is free. Do not start a Docker suite: stop and" \
        'tell the maintainer. Never delete what you do not own or restart Docker Desktop;' \
        'see DEVELOPMENT.md "Keep local development from filling the disk".'
    fi
    ;;
esac

if command -v docker > /dev/null 2>&1; then
  docker system df 2>&1 || printf '%s\n' 'Docker did not answer; its usage is unknown.'
fi
exit 0
