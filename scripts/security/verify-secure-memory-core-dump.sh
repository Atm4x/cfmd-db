#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
pattern_file=/proc/sys/kernel/core_pattern
if [[ ! -r "$pattern_file" ]]; then
  echo "SKIP: kernel core_pattern is unavailable"
  exit 77
fi

pattern="$(cat "$pattern_file")"
if [[ -z "$pattern" || "$pattern" == \|* || "$pattern" == /* ]]; then
  echo "SKIP: core_pattern does not create a directly discoverable core in cwd: $pattern"
  exit 77
fi

cd "$root"
cargo build --release --offline -p cfmd-secure-memory --example core_dump_probe
probe="$root/${CARGO_TARGET_DIR:-target}/release/examples/core_dump_probe"
if [[ ! -x "$probe" ]]; then
  echo "FAIL: probe binary not found: $probe" >&2
  exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
(
  cd "$tmp"
  ulimit -c unlimited
  set +e
  "$probe" >markers.txt 2>&1
  status=$?
  set -e
  if [[ $status -eq 0 ]]; then
    echo "FAIL: probe should terminate by abort" >&2
    exit 1
  fi
)

mapfile -t cores < <(find "$tmp" -maxdepth 1 -type f -name 'core*' -print)
if [[ ${#cores[@]} -ne 1 ]]; then
  echo "SKIP: expected one discoverable core file, found ${#cores[@]}"
  exit 77
fi

control_hex="$(sed -n 's/^CONTROL_MARKER_HEX=//p' "$tmp/markers.txt")"
secure_hex="$(sed -n 's/^SECURE_MARKER_HEX=//p' "$tmp/markers.txt")"
if [[ ${#control_hex} -ne 64 || ${#secure_hex} -ne 64 ]]; then
  echo "FAIL: probe did not emit valid marker metadata" >&2
  cat "$tmp/markers.txt" >&2
  exit 1
fi

python3 - "${cores[0]}" "$control_hex" "$secure_hex" <<'PY'
from pathlib import Path
import sys

core = Path(sys.argv[1]).read_bytes()
control = bytes.fromhex(sys.argv[2])
secure = bytes.fromhex(sys.argv[3])

if control not in core:
    raise SystemExit("FAIL: ordinary control marker is absent; core is not a valid dump-policy control")
if secure in core:
    raise SystemExit("FAIL: secure marker leaked into core dump")
print("PASS: control marker present and secure marker absent from core dump")
PY
