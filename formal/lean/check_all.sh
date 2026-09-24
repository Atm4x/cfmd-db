#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
LEAN_BIN="${LEAN_BIN:-lean}"
(
  cd "$HERE"
  "$LEAN_BIN" CFMD/Publication.lean
  "$LEAN_BIN" CFMD/SurfaceKernel.lean
)
python3 "$HERE/check_refinement.py"
python3 "$HERE/check_surface_refinement.py"
printf 'CFMD Lean mechanization + production refinement: PASS\n'
