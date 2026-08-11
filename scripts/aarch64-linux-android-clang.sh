#!/usr/bin/env bash
# Cargo linker wrapper for aarch64-linux-android.
# Finds NDK clang (API 24) and execs it with the original args.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "${ROOT}/scripts/android-env.sh" >/dev/null

exec "${CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER}" "$@"
