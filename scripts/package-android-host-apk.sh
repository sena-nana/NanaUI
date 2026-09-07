#!/usr/bin/env bash
# Package a prebuilt libnana_android_host.so into a signed debug APK (NativeActivity).
#
# Why not cargo-apk: cargo-apk 0.10's toml 0.5 parser rejects this workspace's
# multiline inline tables in Cargo.toml. This script wraps the .so with SDK
# build-tools instead.
#
# Usage (from repo root):
#   source scripts/android-env.sh
#   ./scripts/check-android-arm64.sh --build --dist
#   ./scripts/package-android-host-apk.sh
#
# Output (default):
#   target-android/apk/nana-android-host-debug.apk
#
# Does NOT install or launch on a device. KeyEvent evidence still needs adb device.

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
PKG="app.nanaui.host"
APP_LABEL="NanaUI"
LIB_NAME="nana_android_host"
MIN_SDK=24
TARGET_SDK=34

if [[ ! -f "${SO}" ]]; then
  echo "package-android-host-apk: missing ${SO}" >&2
  echo "  run: ./scripts/check-android-arm64.sh --build --dist" >&2
  exit 1
fi

BUILD_TOOLS=""
for d in "${ANDROID_HOME}/build-tools/"*; do
  if [[ -x "${d}/aapt" && -x "${d}/zipalign" && -x "${d}/apksigner" ]]; then
    BUILD_TOOLS="${d}"
  fi
done
if [[ -z "${BUILD_TOOLS}" ]]; then
  echo "package-android-host-apk: Android SDK build-tools not found under ${ANDROID_HOME}/build-tools" >&2
  echo "  install: sdkmanager --sdk_root=\"\$ANDROID_HOME\" \"build-tools;34.0.0\"" >&2
  exit 1
fi

ANDROID_JAR="${ANDROID_HOME}/platforms/android-${TARGET_SDK}/android.jar"
if [[ ! -f "${ANDROID_JAR}" ]]; then
  echo "package-android-host-apk: missing ${ANDROID_JAR}" >&2
  exit 1
fi

WORKDIR="${OUT_DIR}/work"
ASSET_ROOT="${WORKDIR}/assets"
rm -rf "${WORKDIR}"
mkdir -p "${ASSET_ROOT}/lib/arm64-v8a" "${WORKDIR}/res/values" "${OUT_DIR}"

STAGED_SO="${ASSET_ROOT}/lib/arm64-v8a/lib${LIB_NAME}.so"
cp "${SO}" "${STAGED_SO}"

# Strip unconditionally, even though [profile.dist] already sets strip="symbols".
# This is the only gate on the path from a .so to an installable APK, and it has
# to hold when someone hands us a hand-built or dev-profile artifact via
# PACKAGE_SO. A dev .so carries ~390 MB of DWARF plus ~48 MB of symbol tables.
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

MANIFEST="${WORKDIR}/AndroidManifest.xml"
cat >"${MANIFEST}" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="${PKG}"
    android:versionCode="1"
    android:versionName="0.1.0">
    <uses-sdk android:minSdkVersion="${MIN_SDK}" android:targetSdkVersion="${TARGET_SDK}" />
    <uses-feature android:name="android.hardware.vulkan.level" android:required="false" />
    <application
        android:label="@string/app_name"
        android:hasCode="false"
        android:extractNativeLibs="false">
        <activity
            android:name="android.app.NativeActivity"
            android:label="@string/app_name"
            android:exported="true"
            android:configChanges="orientation|keyboardHidden|screenSize|smallestScreenSize|screenLayout|uiMode"
            android:launchMode="singleTask">
            <meta-data android:name="android.app.lib_name" android:value="${LIB_NAME}" />
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
EOF

cat >"${WORKDIR}/res/values/strings.xml" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<resources>
    <string name="app_name">${APP_LABEL}</string>
</resources>
EOF

UNALIGNED="${WORKDIR}/unaligned.apk"
ALIGNED="${WORKDIR}/aligned.apk"
FINAL="${OUT_DIR}/${APK_NAME}"

# -0 .so stores the library uncompressed. This is required, not an
# optimisation: with extractNativeLibs="false" the loader mmaps the .so
# straight out of the APK, and a deflated entry cannot be mapped — the install
# fails with "Failed to extract native libraries".
"${BUILD_TOOLS}/aapt" package \
  -f \
  -0 .so \
  -M "${MANIFEST}" \
  -S "${WORKDIR}/res" \
  -I "${ANDROID_JAR}" \
  -F "${UNALIGNED}" \
  "${ASSET_ROOT}"

# -p page-aligns the .so so it maps straight out of the APK; with
# extractNativeLibs="false" that removes the second copy under /data.
"${BUILD_TOOLS}/zipalign" -p -f 4 "${UNALIGNED}" "${ALIGNED}"

KEYSTORE="${OUT_DIR}/debug.keystore"
if [[ ! -f "${KEYSTORE}" ]]; then
  keytool -genkeypair \
    -keystore "${KEYSTORE}" \
    -storepass android \
    -keypass android \
    -alias androiddebugkey \
    -keyalg RSA \
    -keysize 2048 \
    -validity 10000 \
    -dname "CN=Android Debug,O=Android,C=US" \
    >/dev/null
fi

"${BUILD_TOOLS}/apksigner" sign \
  --ks "${KEYSTORE}" \
  --ks-pass pass:android \
  --key-pass pass:android \
  --ks-key-alias androiddebugkey \
  --out "${FINAL}" \
  "${ALIGNED}"

"${BUILD_TOOLS}/apksigner" verify --print-certs "${FINAL}" >/dev/null

echo "package-android-host-apk: OK"
echo "  apk: ${FINAL}"
echo "  so:  ${SO}"
echo "  size: $(wc -c <"${FINAL}") bytes (.so ${AFTER_BYTES} bytes stripped)"
echo "  note: not installed — no device claim. When a device exists:"
echo "    adb install -r ${FINAL}"
