#!/usr/bin/env bash
# Cross-check NanaUI crates for HarmonyOS / OpenHarmony ARM64
# (`aarch64-unknown-linux-ohos`). Check only: OHOS is a compile tier with no
# host in this repository (docs/reference/ohos.md), so there is no .so to build.
#
# Usage:
#   rustup target add aarch64-unknown-linux-ohos
#   ./scripts/check-ohos-arm64.sh
#
# Without the OHOS SDK this checks every crate that needs no OHOS sysroot.
# `fetch` (ureq → rustls → ring) compiles C against libc headers even under
# `cargo check`, so the crates behind it are checked only with the SDK:
#   export OHOS_NDK_HOME=/path/to/ohos-sdk/linux   # contains native/llvm, native/sysroot
# or set CC_aarch64_unknown_linux_ohos yourself.
#
# nana-ui-runtime's stacker still assembles one file without the SDK: psm's
# `aarch_aapcs64.s` includes only psm's own headers, so the host `clang`
# assembles it for the OHOS target. A host gcc cannot (it is x86 only), which
# is why a bare `cc` fallback fails on it.
#
# Optional: export CARGO_TARGET_DIR=$PWD/target-ohos  (the default here)

set -eo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

TARGET=aarch64-unknown-linux-ohos
if [[ $# -gt 0 ]]; then
  echo "check-ohos-arm64: unknown argument $1 (check only; see docs/reference/ohos.md)" >&2
  exit 2
fi

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  export CARGO_TARGET_DIR="${ROOT}/target-ohos"
fi
mkdir -p "${CARGO_TARGET_DIR}"

# The SDK root has `native/`; accept the `native` directory itself too.
if [[ -z "${CC_aarch64_unknown_linux_ohos:-}" && -n "${OHOS_NDK_HOME:-}" ]]; then
  NATIVE="${OHOS_NDK_HOME}"
  [[ -d "${NATIVE}/native" ]] && NATIVE="${NATIVE}/native"
  if [[ ! -x "${NATIVE}/llvm/bin/clang" || ! -d "${NATIVE}/sysroot" ]]; then
    echo "check-ohos-arm64: OHOS_NDK_HOME=${OHOS_NDK_HOME} has no native/llvm/bin/clang and native/sysroot" >&2
    exit 1
  fi
  export CC_aarch64_unknown_linux_ohos="${NATIVE}/llvm/bin/clang"
  export AR_aarch64_unknown_linux_ohos="${NATIVE}/llvm/bin/llvm-ar"
  export CFLAGS_aarch64_unknown_linux_ohos="--target=${TARGET} --sysroot=${NATIVE}/sysroot ${CFLAGS_aarch64_unknown_linux_ohos:-}"
fi

# A compiler from the caller or the SDK has the sysroot `fetch` needs.
OHOS_SYSROOT_CC="${CC_aarch64_unknown_linux_ohos:-}"

if [[ -z "${OHOS_SYSROOT_CC}" ]]; then
  if ! command -v clang >/dev/null 2>&1; then
    echo "check-ohos-arm64: no OHOS C compiler and no host clang; psm (stacker, under nana-ui-runtime) must assemble aarch_aapcs64.s for ${TARGET}" >&2
    exit 1
  fi
  # cc-rs passes clang `--target=${TARGET}`; the file needs no sysroot.
  export CC_aarch64_unknown_linux_ohos=clang
  # Pair it with an LLVM archiver when there is one; otherwise cc-rs uses `ar`.
  LLVM_AR="$(command -v llvm-ar 2>/dev/null || true)"
  if [[ -z "${LLVM_AR}" ]]; then
    CLANG_DIR="$(dirname "$(readlink -f "$(command -v clang)")")"
    [[ -x "${CLANG_DIR}/llvm-ar" ]] && LLVM_AR="${CLANG_DIR}/llvm-ar"
  fi
  [[ -n "${LLVM_AR}" ]] && export AR_aarch64_unknown_linux_ohos="${LLVM_AR}"
  echo "check-ohos-arm64: no OHOS SDK; host clang assembles psm's aarch_aapcs64.s"
fi

echo "check-ohos-arm64: target=${TARGET} CARGO_TARGET_DIR=${CARGO_TARGET_DIR}"

run_crate() {
  local crate="$1"
  shift
  echo "---- cargo check -p ${crate} --target ${TARGET} $* ----"
  cargo check -p "${crate}" --target "${TARGET}" --locked "$@"
}

# Nothing below compiles C against the OHOS sysroot.
run_crate nana-diagnostics
run_crate nana-ui-core
run_crate nana-ui-input
run_crate nana-text
run_crate nana-text --tests
run_crate nana-ui-css
run_crate nana-ui-charts
run_crate nana-ui-diagram
run_crate nana-ui-runtime
run_crate nana-ui-scene
run_crate nana-gpu
run_crate nana-frame-exchange
run_crate nana-ui-platform --no-default-features --features clipboard
run_crate nana-ui-platform --no-default-features --features clipboard --tests
run_crate nana-ui --lib
run_crate nana-ui --lib --no-default-features --features components,bundled-fonts

if [[ -n "${OHOS_SYSROOT_CC}" ]]; then
  echo "check-ohos-arm64: C compiler ${OHOS_SYSROOT_CC}; checking the fetch / GPU path"
  run_crate nana-ui-platform
  run_crate nana-ui --lib --features gpu,components,bundled-fonts
else
  echo "check-ohos-arm64: no OHOS C compiler; fetch / GPU path not checked (set OHOS_NDK_HOME)"
fi

echo "check-ohos-arm64: OK"
