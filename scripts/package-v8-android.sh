#!/usr/bin/env bash
# After V8_FROM_SOURCE=1 cargo build -p nana-js-v8 --features engine \
#   --target aarch64-linux-android --release
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET=aarch64-linux-android
GN_OUT="${CARGO_TARGET_DIR:-$ROOT/target}/${TARGET}/release/gn_out"
OUT="${1:-$ROOT/dist/v8}"
mkdir -p "$OUT"
gzip -9c "$GN_OUT/obj/librusty_v8.a" > "$OUT/librusty_v8_release_${TARGET}.a.gz"
cp "$GN_OUT/src_binding.rs" "$OUT/src_binding_release_${TARGET}.rs"
echo "package-v8-android: wrote $OUT"
