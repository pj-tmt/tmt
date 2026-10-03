#!/bin/sh

real_cargo=${TMT_NATIVE_REAL_CARGO:-}
if [ -z "$real_cargo" ]; then
  printf '%s\n' 'TMT_NATIVE_REAL_CARGO must be an absolute executable path.' >&2
  exit 2
fi

case "$real_cargo" in
  /*) ;;
  *)
    printf '%s\n' 'TMT_NATIVE_REAL_CARGO must be an absolute executable path.' >&2
    exit 2
    ;;
esac

if [ ! -x "$real_cargo" ]; then
  printf '%s\n' "TMT_NATIVE_REAL_CARGO is not executable: $real_cargo" >&2
  exit 2
fi

# This wrapper is the archive-build boundary; ordinary Cargo keeps Colab's dev fallback.
# The Colab-owned build script validates the supplied directory's complete inventory.
if [ "${1:-}" = build ] && [ "${TMT_NATIVE_PRODUCT:-}" = colab ]; then
  case "${TMT_COLAB_APP_DIR:-}" in
    /*) test -d "$TMT_COLAB_APP_DIR" || {
      printf '%s\n' 'Colab release build requires an existing TMT_COLAB_APP_DIR.' >&2
      exit 2
    } ;;
    *)
      printf '%s\n' 'Colab release build requires an absolute TMT_COLAB_APP_DIR.' >&2
      exit 2
      ;;
  esac
fi

for argument in "$@"; do
  if [ "$argument" = '--locked' ] || [ "$argument" = '--frozen' ]; then
    exec "$real_cargo" "$@"
  fi
done

case "${1:-}" in
  build|metadata)
    exec "$real_cargo" "$@" --locked
    ;;
  *)
    exec "$real_cargo" "$@"
    ;;
esac
