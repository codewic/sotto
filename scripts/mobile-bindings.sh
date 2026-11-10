#!/usr/bin/env bash
# Regenerate the Swift and Kotlin bindings for the mobile apps from the Rust core.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
cargo build -p sotto-ffi
lib="target/debug/libsotto_ffi.so"
[ -f "$lib" ] || lib="target/debug/libsotto_ffi.dylib"
cargo run -q -p sotto-ffi --bin uniffi-bindgen -- generate --library "$lib" --language kotlin --language swift --out-dir apps/mobile/bindings
echo "Bindings written to apps/mobile/bindings"
