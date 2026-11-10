#!/usr/bin/env bash
# Build the release binaries twice in separate clean trees and compare hashes.
# Anyone can run this against a tagged release and check the result matches the published SHA256SUMS.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Fixed inputs: toolchain from rust-toolchain.toml, dependency versions from Cargo.lock,
# timestamps from the last commit (or a constant), and build paths remapped away.
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct 2>/dev/null || echo 1700000000)}"
export CARGO_INCREMENTAL=0
bins=(sotto-agent sotto-relay sotto-drop sotto-render sotto-directory)

build() {
  local dir="$1"
  mkdir -p "$dir/src"
  rsync -a --exclude target --exclude node_modules --exclude .dev-data "$root/" "$dir/src/"
  (
    cd "$dir/src"
    RUSTFLAGS="--remap-path-prefix=$HOME/.cargo=/cargo --remap-path-prefix=$dir/src=/build -C codegen-units=1" \
      cargo build --release --locked -q $(printf -- '--bin %s ' "${bins[@]}")
  )
  (cd "$dir/src/target/release" && sha256sum "${bins[@]}") > "$dir/SHA256SUMS"
}

echo "Building copy A…"
build "$work/a"
echo "Building copy B…"
build "$work/b"

if diff -u "$work/a/SHA256SUMS" "$work/b/SHA256SUMS"; then
  echo "Reproducible: both builds are byte-identical."
  cat "$work/a/SHA256SUMS"
else
  echo "Builds differ. Investigate before publishing." >&2
  exit 1
fi
