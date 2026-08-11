#!/usr/bin/env bash
# Pixel similarity gate for NanaUI snapshots / evidence PNGs.
#
# ImageMagick `compare -metric SSIM` on this toolchain reports a *distortion*
# (lower is better). The parenthesized value is the normalized distortion in
# [0, 1]. We define:
#
#   similarity = 1.0 - normalized_distortion
#
# Default threshold: similarity >= 0.98  (normalized distortion <= 0.02).
# Override with NANA_PIXEL_SSIM_MIN (e.g. 0.98). Documented so the gate is not
# an undocumented gut feel.
#
# Usage:
#   scripts/pixel_ssim_compare.sh <baseline.png> <candidate.png>
#   scripts/pixel_ssim_compare.sh --dir <baseline_dir> <candidate_dir>
#
# Exit 0 when every pair passes; 1 on failure / tool error.

set -euo pipefail

MIN="${NANA_PIXEL_SSIM_MIN:-0.98}"

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
  local pass
  pass="$(python3 -c "import sys; sys.exit(0 if float('$sim') + 1e-12 >= float('$MIN') else 1)" && echo yes || echo no)"

  printf 'SSIM %s vs %s: similarity=%.6f (norm_dist=%.6f) threshold=%.2f -> %s\n' \
    "$(basename "$baseline")" "$(basename "$candidate")" "$sim" "$norm" "$MIN" \
    "$([[ $pass == yes ]] && echo PASS || echo FAIL)"

  [[ $pass == yes ]]
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
