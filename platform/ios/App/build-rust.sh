#!/bin/sh
# Xcode "Build Rust host" phase: build libnana_ios_host.a for the platform and
# configuration being built, and copy it where the target links it from.
set -eu

export PATH="${HOME}/.cargo/bin:${PATH}"
ROOT="${SRCROOT}/../../.."
case "${PLATFORM_NAME}" in
  iphonesimulator) TARGET=aarch64-apple-ios-sim ;;
  *) TARGET=aarch64-apple-ios ;;
esac
if [ "${CONFIGURATION}" = "Release" ]; then
  PROFILE=dist
  PROFILE_DIR=dist
else
  PROFILE=dev
  PROFILE_DIR=debug
fi
TARGET_DIR="${CARGO_TARGET_DIR:-${ROOT}/target}"

# Xcode's SDK and deployment variables would leak into cargo's host builds
# (build scripts, proc macros) and point them at the iOS SDK.
env -u SDKROOT -u IPHONEOS_DEPLOYMENT_TARGET -u LIBRARY_PATH \
  cargo build --manifest-path "${ROOT}/Cargo.toml" -p nana-ios-host \
  --target "${TARGET}" --profile "${PROFILE}" --locked

mkdir -p "${DERIVED_FILE_DIR}/rust"
cp "${TARGET_DIR}/${TARGET}/${PROFILE_DIR}/libnana_ios_host.a" "${DERIVED_FILE_DIR}/rust/"
