#!/usr/bin/env bash
# Package a prebuilt libnana_android_host.so into a signed debug APK (GameActivity).
#
# GameActivity + GameTextInput Java classes come from
# androidx.games:games-activity:4.4.0 via Gradle. cargo-apk 0.10 cannot parse
# this workspace's root Cargo.toml, and NativeActivity has no InputConnection.
# Do not enable GameActivity prefab C++ glue.
#
# Usage (from repo root):
#   source scripts/android-env.sh
#   ./scripts/check-android-arm64.sh --build --dist
#   ./scripts/package-android-host-apk.sh
#
# Output (default):
#   target-android/apk/nana-android-host-debug.apk
#
# Does NOT install or launch on a device. CJK IME / TalkBack / clipboard
# evidence still needs a real device (see docs/android.md).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

if [[ -z "${BASH_VERSION:-}" ]]; then
  echo "package-android-host-apk: run under bash" >&2
  exit 1
fi

if [[ -z "${ANDROID_HOME:-}" || -z "${CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER:-}" ]]; then
  # shellcheck disable=SC1091
  source "${ROOT}/scripts/android-env.sh"
fi

TARGET_DIR="${CARGO_TARGET_DIR:-${ROOT}/target-android}"
SO="${PACKAGE_SO:-${TARGET_DIR}/aarch64-linux-android/dist/libnana_android_host.so}"
# The .so is stripped below, so this bounds the *stripped* artifact. A dev-profile
# .so is ~495 MB and ~57 MB even after stripping; a dist one is well under this.
MAX_SO_BYTES="${PACKAGE_MAX_SO_BYTES:-62914560}"  # 60 MiB
OUT_DIR="${PACKAGE_OUT_DIR:-${TARGET_DIR}/apk}"
APK_NAME="${PACKAGE_APK_NAME:-nana-android-host-debug.apk}"
APP_DIR="${ROOT}/platform/android/app"
LIB_NAME="nana_android_host"

if [[ ! -f "${SO}" ]]; then
  echo "package-android-host-apk: missing ${SO}" >&2
  echo "  run: ./scripts/check-android-arm64.sh --build --dist" >&2
  exit 1
fi

if [[ -z "${ANDROID_HOME:-}" || ! -d "${ANDROID_HOME}" ]]; then
  echo "package-android-host-apk: ANDROID_HOME is not set" >&2
  exit 1
fi

GRADLE_BIN="${GRADLE_BIN:-$(command -v gradle || true)}"
if [[ -z "${GRADLE_BIN}" ]]; then
  echo "package-android-host-apk: gradle not found on PATH" >&2
  echo "  GameActivity APK needs Gradle for androidx.games:games-activity:4.4.0" >&2
  echo "  install Gradle 8+, or set GRADLE_BIN" >&2
  exit 1
fi

mkdir -p "${OUT_DIR}" "${APP_DIR}/src/main/jniLibs/arm64-v8a"
STAGED_SO="${APP_DIR}/src/main/jniLibs/arm64-v8a/lib${LIB_NAME}.so"
cp "${SO}" "${STAGED_SO}"

# Strip unconditionally. This is the only gate on the path from a .so to an
# installable APK, and it has to hold when someone hands us a hand-built or
# dev-profile artifact via PACKAGE_SO.
LLVM_STRIP="${LLVM_STRIP:-${ANDROID_NDK_HOME:-}/toolchains/llvm/prebuilt/$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64/bin/llvm-strip}"
if [[ ! -x "${LLVM_STRIP}" ]]; then
  LLVM_STRIP="$(command -v llvm-strip || true)"
fi
if [[ -z "${LLVM_STRIP}" || ! -x "${LLVM_STRIP}" ]]; then
  echo "package-android-host-apk: llvm-strip not found" >&2
  echo "  set LLVM_STRIP=/path/to/llvm-strip, or source scripts/android-env.sh" >&2
  exit 1
fi
BEFORE_BYTES="$(wc -c <"${STAGED_SO}")"
"${LLVM_STRIP}" --strip-all "${STAGED_SO}"
AFTER_BYTES="$(wc -c <"${STAGED_SO}")"
echo "package-android-host-apk: stripped .so ${BEFORE_BYTES} -> ${AFTER_BYTES} bytes"

if (( AFTER_BYTES > MAX_SO_BYTES )); then
  echo "package-android-host-apk: stripped .so is ${AFTER_BYTES} bytes, over the ${MAX_SO_BYTES} byte budget" >&2
  echo "  this almost always means a dev-profile build: use --dist" >&2
  echo "  raise PACKAGE_MAX_SO_BYTES only with a deliberate reason" >&2
  exit 1
fi

# .properties reads backslashes as escapes (Windows ANDROID_HOME).
cat >"${APP_DIR}/local.properties" <<EOF
sdk.dir=$(printf '%s' "${ANDROID_HOME}" | tr '\\' /)
EOF

(
  cd "${APP_DIR}"
  "${GRADLE_BIN}" --no-daemon assembleDebug
)

GRADLE_APK="${APP_DIR}/build/outputs/apk/debug/nana-android-host-debug.apk"
if [[ ! -f "${GRADLE_APK}" ]]; then
  echo "package-android-host-apk: missing ${GRADLE_APK}" >&2
  exit 1
fi

FINAL="${OUT_DIR}/${APK_NAME}"
cp "${GRADLE_APK}" "${FINAL}"

echo "package-android-host-apk: OK"
echo "  apk: ${FINAL}"
echo "  so:  ${SO}"
echo "  size: $(wc -c <"${FINAL}") bytes (.so ${AFTER_BYTES} bytes stripped)"
echo "  activity: app.nanaui.host.NanaActivity (GameActivity)"
echo "  note: not installed — no device claim. When a device exists:"
echo "    adb install -r ${FINAL}"
echo "    adb shell am start -n app.nanaui.host/.NanaActivity"
