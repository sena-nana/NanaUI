#!/usr/bin/env bash
# Source this file before Android ARM64 cargo builds:
#   source scripts/android-env.sh
# Prefer bash. Under zsh, `source` still works if BASH_SOURCE is emulated below.
#
# Resolves ANDROID_HOME / ANDROID_NDK_HOME and exports the aarch64 linker
# toolchain variables Cargo + cc expect. BINDGEN_EXTRA_CLANG_ARGS remains for
# remaining C deps; rquickjs/QuickJS bindgen is gone with that engine.

set -euo pipefail

# Requires bash (`source` from bash, or invoked by scripts/*.sh shebangs).
if [[ -z "${BASH_VERSION:-}" ]]; then
  echo "android-env: source from bash (e.g. bash -c 'source scripts/android-env.sh')" >&2
  return 1 2>/dev/null || exit 1
fi
_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

_resolve_sdk() {
  if [[ -n "${ANDROID_HOME:-}" && -d "${ANDROID_HOME}" ]]; then
    echo "${ANDROID_HOME}"
    return
  fi
  if [[ -n "${ANDROID_SDK_ROOT:-}" && -d "${ANDROID_SDK_ROOT}" ]]; then
    echo "${ANDROID_SDK_ROOT}"
    return
  fi
  local candidates=(
    "${HOME}/Android/Sdk"
    "/opt/homebrew/share/android-commandlinetools"
    "${HOME}/Library/Android/sdk"
  )
  local c
  for c in "${candidates[@]}"; do
    if [[ -d "${c}" ]]; then
      echo "${c}"
      return
    fi
  done
  return 1
}

_resolve_ndk() {
  if [[ -n "${ANDROID_NDK_HOME:-}" && -d "${ANDROID_NDK_HOME}" ]]; then
    echo "${ANDROID_NDK_HOME}"
    return
  fi
  if [[ -n "${ANDROID_NDK_ROOT:-}" && -d "${ANDROID_NDK_ROOT}" ]]; then
    echo "${ANDROID_NDK_ROOT}"
    return
  fi
  local sdk="$1"
  if [[ -d "${sdk}/ndk" ]]; then
    local best
    best="$(ls -1 "${sdk}/ndk" 2>/dev/null | sort -V | tail -1 || true)"
    if [[ -n "${best}" && -d "${sdk}/ndk/${best}" ]]; then
      echo "${sdk}/ndk/${best}"
      return
    fi
  fi
  return 1
}

# GitHub Actions `setup-ndk` provides ANDROID_NDK_HOME without a full SDK.
if [[ -n "${ANDROID_NDK_HOME:-}" && -d "${ANDROID_NDK_HOME}" ]]; then
  export ANDROID_NDK_HOME
else
  if ! ANDROID_HOME="$(_resolve_sdk)"; then
    echo "android-env: ANDROID_HOME not found. Run scripts/setup-android-ndk.sh first." >&2
    return 1 2>/dev/null || exit 1
  fi
  export ANDROID_HOME
  export ANDROID_SDK_ROOT="${ANDROID_HOME}"
  if ! ANDROID_NDK_HOME="$(_resolve_ndk "${ANDROID_HOME}")"; then
    echo "android-env: NDK not found under ${ANDROID_HOME}/ndk. Run scripts/setup-android-ndk.sh." >&2
    return 1 2>/dev/null || exit 1
  fi
  export ANDROID_NDK_HOME
fi
if [[ -n "${ANDROID_HOME:-}" ]]; then
  export ANDROID_SDK_ROOT="${ANDROID_HOME}"
fi
export ANDROID_NDK_ROOT="${ANDROID_NDK_HOME}"

_HOST_TAG=""
for _tag in darwin-arm64 darwin-x86_64 linux-x86_64; do
  if [[ -d "${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/${_tag}" ]]; then
    _HOST_TAG="${_tag}"
    break
  fi
done
if [[ -z "${_HOST_TAG}" ]]; then
  echo "android-env: no NDK prebuilt host toolchain under ${ANDROID_NDK_HOME}" >&2
  return 1 2>/dev/null || exit 1
fi

_NDK_BIN="${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/${_HOST_TAG}/bin"
_NDK_SYSROOT="${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/${_HOST_TAG}/sysroot"
_API="${ANDROID_API_LEVEL:-24}"
_CLANG="${_NDK_BIN}/aarch64-linux-android${_API}-clang"
_CLANGXX="${_NDK_BIN}/aarch64-linux-android${_API}-clang++"

if [[ ! -x "${_CLANG}" ]]; then
  echo "android-env: missing ${_CLANG}" >&2
  return 1 2>/dev/null || exit 1
fi

export PATH="${_NDK_BIN}:${PATH}"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${_CLANG}"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_AR="${_NDK_BIN}/llvm-ar"
export CC_aarch64_linux_android="${_CLANG}"
export CXX_aarch64_linux_android="${_CLANGXX}"
export AR_aarch64_linux_android="${_NDK_BIN}/llvm-ar"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=${_NDK_SYSROOT} -target aarch64-linux-android${_API}"

if [[ -z "${LIBCLANG_PATH:-}" ]]; then
  if [[ -d /opt/homebrew/opt/llvm/lib ]]; then
    export LIBCLANG_PATH=/opt/homebrew/opt/llvm/lib
  elif [[ -d /Library/Developer/CommandLineTools/usr/lib ]]; then
    export LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib
  fi
fi

if [[ -z "${JAVA_HOME:-}" ]]; then
  if [[ -d /opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home ]]; then
    export JAVA_HOME=/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home
  elif [[ -d /opt/homebrew/opt/openjdk@21 ]]; then
    export JAVA_HOME=/opt/homebrew/opt/openjdk@21
  fi
fi

echo "android-env: ANDROID_HOME=${ANDROID_HOME:-}"
echo "android-env: ANDROID_NDK_HOME=${ANDROID_NDK_HOME}"
echo "android-env: linker=${CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER}"
echo "android-env: repo=${_REPO_ROOT}"
