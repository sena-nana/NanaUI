#!/bin/sh
# Xcode "Build Rust host" phase: build the app's Rust staticlib for the
# platform and configuration being built, and copy it where the target
# links it from.
#
# An app sets, in its target's build settings or the phase's environment:
#   NANA_RUST_PACKAGE   the crate that builds the staticlib (nana-ios-probe)
#   NANA_RUST_LIB       its library name (nana_ios_probe)
#   NANA_CARGO_MANIFEST the workspace's Cargo.toml (this NanaUI checkout)
set -eu

export PATH="${HOME}/.cargo/bin:${PATH}"
ROOT="${SRCROOT}/../../.."
PACKAGE="${NANA_RUST_PACKAGE:-nana-ios-probe}"
LIB="${NANA_RUST_LIB:-nana_ios_probe}"
MANIFEST="${NANA_CARGO_MANIFEST:-${ROOT}/Cargo.toml}"
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
TARGET_DIR="${CARGO_TARGET_DIR:-$(dirname "${MANIFEST}")/target}"

# Xcode's SDK variables would leak into cargo's host builds (build scripts,
# proc macros) and point them at the iOS SDK. IPHONEOS_DEPLOYMENT_TARGET
# stays: only iOS targets read it, and C dependencies (ring) need it to
# match the app's deployment target.
env -u SDKROOT -u LIBRARY_PATH \
  cargo build --manifest-path "${MANIFEST}" -p "${PACKAGE}" \
  --target "${TARGET}" --profile "${PROFILE}" --locked

mkdir -p "${DERIVED_FILE_DIR}/rust"
cp "${TARGET_DIR}/${TARGET}/${PROFILE_DIR}/lib${LIB}.a" "${DERIVED_FILE_DIR}/rust/"
