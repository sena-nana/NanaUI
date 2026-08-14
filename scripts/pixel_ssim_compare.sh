#!/usr/bin/env bash
# Optional pixel-similarity diagnostic for NanaUI snapshots / evidence PNGs.
#
# ImageMagick `compare -metric SSIM` on this toolchain reports a *distortion*
# (lower is better). The parenthesized value is the normalized distortion in
# [0, 1]. We define:
#
#   similarity = 1.0 - normalized_distortion
#
# Usage:
#   scripts/pixel_ssim_compare.sh <baseline.png> <candidate.png>
#   scripts/pixel_ssim_compare.sh --dir <baseline_dir> <candidate_dir>
#
# Pixel similarity does not determine rendering correctness and is never a
# promotion gate. Exit 1 only when the diagnostic cannot run (missing files,
# tool errors or unparseable output).

set -euo pipefail

compare_pair() {
  local baseline="$1"
  local candidate="$2"
  if [[ ! -f "$baseline" ]]; then
    echo "FAIL missing baseline: $baseline" >&2
    return 1
  fi
  if [[ ! -f "$candidate" ]]; then
    echo "FAIL missing candidate: $candidate" >&2
    return 1
  fi

  # stderr carries "raw (normalized)"; stdout unused (null: discard).
  local out
  set +e
  out="$(magick compare -metric SSIM "$baseline" "$candidate" null: 2>&1)"
  local rc=$?
  set -e
  # ImageMagick returns 1 when images differ; that is expected for scoring.
  if [[ $rc -gt 1 ]]; then
    echo "FAIL magick compare rc=$rc for $baseline vs $candidate: $out" >&2
    return 1
  fi

  local norm
  norm="$(python3 - "$out" <<'PY'
import re, sys
text = sys.argv[1]
m = re.search(r"\(([-+0-9.eE]+)\)", text)
if not m:
    # Identical sometimes prints bare "0"
    m2 = re.search(r"^([-+0-9.eE]+)", text.strip())
    if not m2:
        print("nan", file=sys.stderr)
        sys.exit(2)
    print(float(m2.group(1)))
else:
    print(float(m.group(1)))
PY
)" || {
    echo "FAIL parse SSIM output for $baseline vs $candidate: $out" >&2
    return 1
  }

  local sim
  sim="$(python3 -c "print(1.0 - float('$norm'))")"
  printf 'SSIM diagnostic %s vs %s: similarity=%.6f (norm_dist=%.6f; non-gating)\n' \
    "$(basename "$baseline")" "$(basename "$candidate")" "$sim" "$norm"
}

if [[ "${1:-}" == "--dir" ]]; then
  base_dir="${2:?baseline dir}"
  cand_dir="${3:?candidate dir}"
  fail=0
  shopt -s nullglob
  maps=("$base_dir"/*.png)
  if [[ ${#maps[@]} -eq 0 ]]; then
    echo "FAIL no PNGs in $base_dir" >&2
    exit 1
  fi
  for b in "${maps[@]}"; do
    name="$(basename "$b")"
    if ! compare_pair "$b" "$cand_dir/$name"; then
      fail=1
    fi
  done
  exit $fail
fi

baseline="${1:?baseline png}"
candidate="${2:?candidate png}"
compare_pair "$baseline" "$candidate"
