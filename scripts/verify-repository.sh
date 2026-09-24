#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Project root should stay project-facing, not pass-artifact-facing.
if find . -maxdepth 1 -type f \( -name 'PASS*' -o -name 'IMPLEMENTATION_REPORT_pass*' -o -name 'HISTORICAL_PROBLEMS_LEDGER_PASS*' \) | grep -q .; then
  echo 'historical pass artifact leaked back into repository root' >&2
  exit 1
fi

for required in \
  README.md SPEC.md Cargo.toml Cargo.lock rust-toolchain.toml lean-toolchain \
  docs/spec/CFMD_CORE_SPEC.md docs/status/HISTORICAL_PROBLEMS_LEDGER.md \
  docs/architecture/ARCHITECTURE.md formal/lean/CFMD/Publication.lean \
  formal/lean/CFMD/SurfaceKernel.lean; do
  test -f "$required" || { echo "missing required file: $required" >&2; exit 1; }
done

echo 'repository layout: PASS'
