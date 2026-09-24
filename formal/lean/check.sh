#!/usr/bin/env bash
set -euo pipefail
LEAN_BIN="${LEAN_BIN:-lean}"
exec "$LEAN_BIN" CFMD/Publication.lean
