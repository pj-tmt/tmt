#!/bin/sh
set -eu

notices_only=false
if [ "${1:-}" = --notices-only ]; then
  notices_only=true
  shift
fi
if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  printf '%s\n' 'Usage: scripts/build-native-artifact.sh [--notices-only] <cargo-dist target> [cli|office|squad|driver-herdr|remote|colab]' >&2
  exit 2
fi
target=$1
product=${2:-cli}
case "$product" in cli|office|squad|driver-herdr|remote|colab) ;; *) printf '%s\n' 'Unknown native product.' >&2; exit 2 ;; esac
repo=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
if [ "$product" = office ]; then
  cd "$repo/typescript"
  corepack pnpm office:build:local 1>&2
  TMT_OFFICE_SPA_DIR="$repo/target/office-spa"
  export TMT_OFFICE_SPA_DIR
fi
if [ "$product" = colab ]; then
  cd "$repo/typescript"
  corepack pnpm@10.33.0 install --frozen-lockfile --ignore-scripts 1>&2
  corepack pnpm@10.33.0 --filter @tmt/colab-app --fail-if-no-match build 1>&2
  TMT_COLAB_APP_DIR="$repo/extensions/tmt-colab/typescript/app/dist"
  export TMT_COLAB_APP_DIR
  # The complete Vite output stays in place until native compilation finishes.
  # build.rs validates the inventory; packaging never accepts the local fallback.
  test -s "$TMT_COLAB_APP_DIR/index.html"
  test -d "$TMT_COLAB_APP_DIR/assets"
  test -s "$TMT_COLAB_APP_DIR/THIRD-PARTY-NOTICES.txt"
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
TMT_NATIVE_PRODUCT=$product
export TMT_NATIVE_PRODUCT
if [ "$notices_only" = false ]; then
  package_id=$(cargo pkgid --locked -p "tmt-$product")
  # Cargo emits either #version or #name@version for a resolved package ID.
  version=${package_id##*#}
  version=${version##*@}
  # Extensions are versioned and tagged independently of the CLI.
  case "$product" in
    cli) tag="v$version" ;;
    *) tag="tmt-$product-v$version" ;;
  esac
  # The CLI carries the independently owned binary through the first standalone
  # Herdr release. Build it once from its package, then let cargo-dist include it
  # without a second bin target.
  if [ "$product" = cli ]; then
    cargo build --locked -p tmt-driver-herdr --bin tmt-driver-herdr \
      --profile dist --target "$target" --target-dir "$repo/rust/target" 1>&2
    mkdir -p target/native-companion
    cp -p "target/$target/dist/tmt-driver-herdr" target/native-companion/tmt-driver-herdr
  fi
  # cargo-dist checks its own version against dist-workspace.toml.
  cd "$repo"
  dist generate --check --target "$target" --tag "$tag" 1>&2
fi
cd "$repo/rust"
mkdir -p target/native-notices
# Registry git clarifications need HTTP in cargo-about 0.9.2, and some crate archives omit their
# license file (cargo-about would then print the SPDX template with placeholder attribution).
# Resolve a vendored local file clarification instead, without mutating Cargo's registry or
# license text. Each row is "crate version file"; about.toml carries the matching
# [crate.clarify] checksum and a __TMT_<CRATE>_LICENSE__ path token.
vendored_licenses='taffy 0.7.7 LICENSE.md
yrs 0.28.0 LICENSE'
cp about.toml target/native-notices/about.toml
while read -r crate version file; do
  awk -v name="$crate" -v version="$version" '
    $0 == "name = \"" name "\"" { count++; getline; if ($0 != "version = \"" version "\"") bad = 1 }
    END { if (count != 1 || bad) exit 1 }
  ' Cargo.lock || { printf '%s\n' "Review $crate notice on version changes." >&2; exit 1; }
  license_path="$repo/rust/licenses/$crate-$version/$file"
  expected_checksum=$(awk -v name="$crate" '
    /^\[/ { active = ($0 ~ "^\\[\\[?" name "\\.clarify[].]") }
    active && /^checksum =/ { gsub(/"/, "", $3); print $3; exit }
  ' about.toml)
  if command -v sha256sum >/dev/null 2>&1; then
    actual_checksum=$(sha256sum < "$license_path")
  else
    actual_checksum=$(shasum -a 256 < "$license_path")
  fi
  test -n "$expected_checksum" && test "${actual_checksum%% *}" = "$expected_checksum" || {
    printf '%s\n' "Vendored $crate license checksum mismatch." >&2
    exit 1
  }
  # Escape first for TOML, then for sed's replacement string (including its delimiter).
  escaped_license_path=$(printf '%s' "$license_path" | sed 's/[\\"]/\\&/g' | sed 's/[\\&|]/\\&/g')
  token="__TMT_$(printf '%s' "$crate" | tr '[:lower:]' '[:upper:]')_LICENSE__"
  sed "s|$token|$escaped_license_path|" target/native-notices/about.toml > target/native-notices/about.toml.next
  mv target/native-notices/about.toml.next target/native-notices/about.toml
done <<EOF_LICENSES
$vendored_licenses
EOF_LICENSES
case "$product" in
  cli) product_manifest="crates/tmt-cli/Cargo.toml" ;;
  driver-herdr) product_manifest="crates/tmt-driver-herdr/Cargo.toml" ;;
  *) product_manifest="../extensions/tmt-$product/rust/tmt-$product/Cargo.toml" ;;
esac
cargo-about generate --manifest-path "$product_manifest" \
  --config target/native-notices/about.toml --target "$target" --locked --offline --fail about.hbs \
  --output-file target/native-notices/THIRD-PARTY-NOTICES.txt 1>&2
if [ "$product" = cli ]; then
  cargo-about generate --manifest-path crates/tmt-driver-herdr/Cargo.toml \
    --config target/native-notices/about.toml --target "$target" --locked --offline --fail about.hbs \
    --output-file target/native-notices/HERDR-NOTICES.txt 1>&2
  cat target/native-notices/HERDR-NOTICES.txt >> target/native-notices/THIRD-PARTY-NOTICES.txt
fi
if [ "$product" = office ]; then
  # Vite owns the inventory of dependencies actually included in the SPA bundle.
  test -s "$TMT_OFFICE_SPA_DIR/THIRD-PARTY-NOTICES.txt"
  cat "$TMT_OFFICE_SPA_DIR/THIRD-PARTY-NOTICES.txt" >> target/native-notices/THIRD-PARTY-NOTICES.txt
fi
if [ "$product" = colab ]; then
  cat "$TMT_COLAB_APP_DIR/THIRD-PARTY-NOTICES.txt" >> target/native-notices/THIRD-PARTY-NOTICES.txt
fi

if [ "$notices_only" = true ]; then exit 0; fi
# Keep diagnostics on stderr and cargo-dist's authoritative manifest on stdout.
# Callers save stdout alongside the archives, then run the independent verifier.
cd "$repo"
dist build --artifacts local --target "$target" --tag "$tag" --output-format=json --no-local-paths
