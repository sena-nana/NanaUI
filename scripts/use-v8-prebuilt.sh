#!/usr/bin/env bash
# source scripts/use-v8-prebuilt.sh [dir]
if [[ -z "${BASH_VERSION:-}" ]]; then
  echo "use-v8-prebuilt: source from bash" >&2
  return 1 2>/dev/null || exit 1
fi
_DIR="$(cd "${1:-dist/v8}" && pwd)"
export RUSTY_V8_ARCHIVE="${_DIR}/librusty_v8_release_aarch64-linux-android.a.gz"
export RUSTY_V8_SRC_BINDING_PATH="${_DIR}/src_binding_release_aarch64-linux-android.rs"
unset V8_FROM_SOURCE || true
echo "use-v8-prebuilt: $RUSTY_V8_ARCHIVE"
