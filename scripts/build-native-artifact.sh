#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  printf '%s\n' 'Usage: scripts/build-native-artifact.sh <cargo-dist target> [cli|office]' >&2
  exit 2
fi
target=$1
product=${2:-cli}
case "$product" in cli|office) ;; *) printf '%s\n' 'Unknown native product.' >&2; exit 2 ;; esac
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
if [ "$product" = office ]; then
  cd "$repo/typescript"
  corepack pnpm office:build:local 1>&2
  TMT_OFFICE_SPA_DIR="$repo/target/office-spa"
  export TMT_OFFICE_SPA_DIR
fi
cd "$repo/rust"
# Resolve the repository toolchain before cargo-dist discovers the generic root
# workspace. Source archives/containers need not contain Git metadata.
selected_toolchain=$(rustup show active-toolchain)
RUSTUP_TOOLCHAIN=${selected_toolchain%% *}
export RUSTUP_TOOLCHAIN

# Developer tools only; do not silently install tools or change release settings.
test "$(cargo-about --version)" = 'cargo-about 0.9.2' || {
  printf '%s\n' 'Native packaging requires cargo-about 0.9.2.' >&2
  exit 2
}
TMT_NATIVE_REAL_CARGO=${TMT_NATIVE_REAL_CARGO:-$(command -v cargo)}
export TMT_NATIVE_REAL_CARGO
CARGO="$repo/scripts/native-cargo.sh"
export CARGO
package_id=$(cargo pkgid --locked -p "tmt-$product")
# Cargo emits either #version or #name@version for a resolved package ID.
version=${package_id##*#}
version=${version##*@}
tag="v$version"
if [ "$product" = office ]; then tag="tmt-office-v$version"; fi
# cargo-dist checks its own version against dist-workspace.toml.
cd "$repo"
dist generate --check --target "$target" --tag "$tag" 1>&2
cd "$repo/rust"
mkdir -p target/native-notices
product_manifest="crates/tmt-cli/Cargo.toml"
if [ "$product" = office ]; then
  product_manifest="../extensions/tmt-office/rust/tmt-office/Cargo.toml"
fi
cargo-about generate --manifest-path "$product_manifest" \
  --config about.toml --target "$target" --locked --offline --fail about.hbs \
  --output-file target/native-notices/THIRD-PARTY-NOTICES.txt 1>&2
if [ "$product" = office ]; then
  # Vite owns the inventory of dependencies actually included in the SPA bundle.
  test -s "$TMT_OFFICE_SPA_DIR/THIRD-PARTY-NOTICES.txt"
  cat "$TMT_OFFICE_SPA_DIR/THIRD-PARTY-NOTICES.txt" >> target/native-notices/THIRD-PARTY-NOTICES.txt
fi

# Keep diagnostics on stderr and cargo-dist's authoritative manifest on stdout.
# Callers save stdout alongside the archives, then run the independent verifier.
cd "$repo"
dist build --artifacts local --target "$target" --tag "$tag" --output-format=json --no-local-paths
