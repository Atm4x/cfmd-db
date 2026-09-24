#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
LEAN_BIN="${LEAN_BIN:-lean}"
cd "$HERE"
"$LEAN_BIN" CFMD/SurfaceKernel.lean
python3 check_surface_refinement.py
printf 'P20 surface-to-kernel mechanization: PASS\n'
