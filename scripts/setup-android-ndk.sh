#!/usr/bin/env bash
# Install Android SDK side-by-side NDK (r27+) into a user-writable SDK root.
# Does not require sudo. Uses Homebrew openjdk@21 + android-commandlinetools when present.
#
# Usage:
#   ./scripts/setup-android-ndk.sh
#   ANDROID_SDK_ROOT=~/Android/Sdk NDK_PACKAGE=ndk;27.2.12479018 ./scripts/setup-android-ndk.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SDK_ROOT="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-${HOME}/Android/Sdk}}"
NDK_PACKAGE="${NDK_PACKAGE:-ndk;27.2.12479018}"

if [[ -z "${JAVA_HOME:-}" ]]; then
  if [[ -d /opt/homebrew/opt/openjdk@21 ]]; then
    export JAVA_HOME=/opt/homebrew/opt/openjdk@21
  fi
fi
if [[ -n "${JAVA_HOME:-}" ]]; then
  export PATH="${JAVA_HOME}/bin:${PATH}"
fi

if ! command -v java >/dev/null 2>&1; then
  echo "setup-android-ndk: Java required. Install with: brew install openjdk@21" >&2
  exit 1
fi

SDKMANAGER=""
for candidate in \
  "$(command -v sdkmanager || true)" \
  /opt/homebrew/share/android-commandlinetools/cmdline-tools/latest/bin/sdkmanager \
  "${SDK_ROOT}/cmdline-tools/latest/bin/sdkmanager"
do
  if [[ -n "${candidate}" && -x "${candidate}" ]]; then
    SDKMANAGER="${candidate}"
    break
  fi
done

if [[ -z "${SDKMANAGER}" ]]; then
  echo "setup-android-ndk: sdkmanager not found. Install: brew install --cask android-commandlinetools" >&2
  exit 1
fi

mkdir -p "${SDK_ROOT}"
echo "setup-android-ndk: SDK_ROOT=${SDK_ROOT}"
echo "setup-android-ndk: installing ${NDK_PACKAGE} (+ platform-tools, android-34)"

yes | "${SDKMANAGER}" --sdk_root="${SDK_ROOT}" --licenses >/tmp/nanaui-sdk-licenses.log 2>&1 || true
yes | "${SDKMANAGER}" --sdk_root="${SDK_ROOT}" \
  "${NDK_PACKAGE}" \
  "platform-tools" \
  "platforms;android-34" \
  "build-tools;34.0.0" 2>&1 | tee /tmp/nanaui-ndk-install.log | tail -20

export ANDROID_HOME="${SDK_ROOT}"
export ANDROID_SDK_ROOT="${SDK_ROOT}"
# shellcheck disable=SC1091
source "${ROOT}/scripts/android-env.sh"

echo "setup-android-ndk: done."
echo "Next: source scripts/android-env.sh && ./scripts/check-android-arm64.sh"
