#!/usr/bin/env bash
# Package a prebuilt libnana_android_host.so into a signed debug APK (NativeActivity).
#
# Why not cargo-apk: cargo-apk 0.10's toml 0.5 parser rejects this workspace's
# multiline inline tables in Cargo.toml. This script wraps the .so with SDK
# build-tools instead.
#
# Usage (from repo root):
#   source scripts/android-env.sh
#   ./scripts/check-android-arm64.sh --build
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
SO="${PACKAGE_SO:-${TARGET_DIR}/aarch64-linux-android/debug/libnana_android_host.so}"
OUT_DIR="${PACKAGE_OUT_DIR:-${TARGET_DIR}/apk}"
APK_NAME="${PACKAGE_APK_NAME:-nana-android-host-debug.apk}"
PKG="app.nanaui.host"
APP_LABEL="NanaUI"
LIB_NAME="nana_android_host"
MIN_SDK=24
TARGET_SDK=34

if [[ ! -f "${SO}" ]]; then
  echo "package-android-host-apk: missing ${SO}" >&2
  echo "  run: ./scripts/check-android-arm64.sh --build" >&2
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

cp "${SO}" "${ASSET_ROOT}/lib/arm64-v8a/lib${LIB_NAME}.so"

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
        android:extractNativeLibs="true">
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

"${BUILD_TOOLS}/aapt" package \
  -f \
  -M "${MANIFEST}" \
  -S "${WORKDIR}/res" \
  -I "${ANDROID_JAR}" \
  -F "${UNALIGNED}" \
  "${ASSET_ROOT}"

"${BUILD_TOOLS}/zipalign" -f 4 "${UNALIGNED}" "${ALIGNED}"

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
echo "  size: $(wc -c <"${FINAL}") bytes"
echo "  note: not installed — no device claim. When a device exists:"
echo "    adb install -r ${FINAL}"
