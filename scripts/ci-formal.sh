#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
if ! command -v lean >/dev/null 2>&1; then
  echo "Lean not found. Install the toolchain pinned by ./lean-toolchain (Lean 4.34.0)." >&2
  exit 127
fi
lean --version
bash formal/lean/check_all.sh
