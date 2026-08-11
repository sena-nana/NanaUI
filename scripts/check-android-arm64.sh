#!/usr/bin/env bash
# Cross-check / build Android ARM64 crates (no WebView, no Blitz).
#
# Usage:
#   source scripts/android-env.sh
#   ./scripts/check-android-arm64.sh
#   ./scripts/check-android-arm64.sh --build
#
# Optional: export CARGO_TARGET_DIR=$PWD/target-android  (recommended on low disk)

set -eo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

if [[ -z "${CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER:-}" ]]; then
  # shellcheck disable=SC1091
  source "${ROOT}/scripts/android-env.sh"
fi

# Prefer an isolated target dir when the workspace `target/` is huge.
if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  export CARGO_TARGET_DIR="${ROOT}/target-android"
fi
mkdir -p "${CARGO_TARGET_DIR}"

TARGET=aarch64-linux-android
MODE=check
if [[ "${1:-}" == "--build" ]]; then
  MODE=build
fi

echo "check-android-arm64: mode=${MODE} target=${TARGET}"
echo "check-android-arm64: CARGO_TARGET_DIR=${CARGO_TARGET_DIR}"

run_crate() {
  local crate="$1"
  shift
  echo "---- cargo ${MODE} -p ${crate} --target ${TARGET} $* ----"
  cargo "${MODE}" -p "${crate}" --target "${TARGET}" --locked "$@"
}

run_crate nana-js-engine
run_crate nana-js-quickjs
run_crate nana-ui-core
run_crate nana-ui-web-api
run_crate nana-ui-platform
run_crate nana-ui --lib
run_crate nana-ui-vue --no-default-features
run_crate nana-android-host

ARTIFACT="${CARGO_TARGET_DIR}/${TARGET}/debug/libnana_android_host.so"
if [[ -f "${ARTIFACT}" ]]; then
  echo "check-android-arm64: artifact ${ARTIFACT} ($(wc -c <"${ARTIFACT}") bytes)"
  file "${ARTIFACT}" || true
elif [[ "${MODE}" == "build" ]]; then
  echo "check-android-arm64: expected ${ARTIFACT} missing" >&2
  exit 1
else
  echo "check-android-arm64: check-only (pass --build for .so evidence)"
fi

echo "check-android-arm64: OK"
