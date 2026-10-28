#!/usr/bin/env bash
# Cross-check / build the iOS ARM64 crates (no WebView, no winit host).
#
# Usage:
#   ./scripts/check-ios-arm64.sh               # cargo check, device target
#   ./scripts/check-ios-arm64.sh --sim         # aarch64-apple-ios-sim
#   ./scripts/check-ios-arm64.sh --build       # dev staticlib
#   ./scripts/check-ios-arm64.sh --build --dist
#
# Needs Xcode with the iOS SDK and `rustup target add aarch64-apple-ios`
# (and `aarch64-apple-ios-sim` for --sim).

set -eo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

TARGET=aarch64-apple-ios
MODE=check
PROFILE=dev
PROFILE_DIR=debug
for arg in "$@"; do
  case "${arg}" in
    --sim) TARGET=aarch64-apple-ios-sim ;;
    --build) MODE=build ;;
    --dist) MODE=build; PROFILE=dist; PROFILE_DIR=dist ;;
    *)
      echo "check-ios-arm64: unknown argument ${arg}" >&2
      echo "  usage: $0 [--sim] [--build] [--dist]" >&2
      exit 2
      ;;
  esac
done

echo "check-ios-arm64: mode=${MODE} profile=${PROFILE} target=${TARGET}"

run_crate() {
  local crate="$1"
  shift
  echo "---- cargo ${MODE} -p ${crate} --target ${TARGET} --profile ${PROFILE} $* ----"
  cargo "${MODE}" -p "${crate}" --target "${TARGET}" --profile "${PROFILE}" --locked "$@"
}

run_crate nana-ui-core
run_crate nana-ui-platform
run_crate nana-ui --lib --no-default-features --features gpu,accesskit-tree,wgpu-interop,wgpu-backends
# What an app on the mobile hosts builds its pages with: declarative views,
# controls, icons and the bundled fonts.
run_crate nana-ui --lib --no-default-features \
  --features gpu,accesskit-tree,wgpu-interop,wgpu-backends,view-macro,controls,icons-tabler,bundled-fonts
run_crate nana-mobile-host

if [[ -d platform/ios ]]; then
  run_crate nana-ios-host
  # The measurement page's staticlib, the one an app links in its place.
  run_crate nana-ios-probe
  ARTIFACT="${CARGO_TARGET_DIR:-target}/${TARGET}/${PROFILE_DIR}/libnana_ios_probe.a"
  if [[ -f "${ARTIFACT}" ]]; then
    echo "check-ios-arm64: artifact ${ARTIFACT} ($(wc -c <"${ARTIFACT}") bytes)"
  elif [[ "${MODE}" == "build" ]]; then
    echo "check-ios-arm64: expected ${ARTIFACT} missing" >&2
    exit 1
  fi
fi

echo "check-ios-arm64: OK"
