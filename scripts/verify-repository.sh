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
  README.md SPEC.md PROJECT_RULES.md Cargo.toml Cargo.lock rust-toolchain.toml lean-toolchain \
  crates/cfmd/Cargo.toml crates/cfmd-derive/Cargo.toml scripts/verify-public-api.sh scripts/verify-cfmd-derive-diagnostics.sh \
  REPOSITORY_MANIFEST.sha256 \
  docs/spec/CFMD_CORE_SPEC.md docs/status/HISTORICAL_PROBLEMS_LEDGER.md \
  docs/status/KERNEL_HOSTILE_LEDGER.md docs/status/PROJECT_STATUS.md docs/status/PRODUCTIZATION_LEDGER.md \
  docs/api/PRODUCT_ROADMAP.md docs/api/CFMD_PYTHON_FACADE_THEORY.md docs/api/ASYNC_ADAPTER_DESIGN.md \
  docs/architecture/ARCHITECTURE.md formal/lean/CFMD/Publication.lean \
  formal/lean/CFMD/Notification.lean \
  formal/lean/CFMD/SecurityAuthority.lean \
  formal/lean/CFMD/SessionLifecycle.lean \
  formal/lean/CFMD/WireProtocol.lean \
  formal/lean/CFMD/HostedBoundary.lean \
  formal/lean/CFMD/LocalTransport.lean \
  formal/lean/CFMD/SingleFile.lean \
  formal/lean/CFMD/DurabilityBackend.lean \
  formal/lean/CFMD/SurfaceKernel.lean; do
  test -f "$required" || { echo "missing required file: $required" >&2; exit 1; }
done

# Literal Rust include!("...") targets are part of the source tree. Catch partial
# GitHub uploads before Cargo emits a less useful compiler error.
python3 - <<'PY'
from pathlib import Path
import re
import sys

root = Path.cwd()
missing = []
pattern = re.compile(r'\binclude!\s*\(\s*"([^"]+)"\s*\)')
for source in root.joinpath('crates').rglob('*.rs'):
    text = source.read_text(encoding='utf-8')
    for rel in pattern.findall(text):
        target = source.parent / rel
        if not target.is_file():
            missing.append((source.relative_to(root), rel, target.relative_to(root)))

if missing:
    for source, rel, target in missing:
        print(f'missing Rust include target: {source}: include!("{rel}") -> {target}', file=sys.stderr)
    raise SystemExit(1)
PY

# cargo-vendor metadata is authoritative. Validate every file listed by each
# .cargo-checksum.json so broad ignore rules or partial uploads fail early.
python3 - <<'PY'
from pathlib import Path
import hashlib
import json
import sys

root = Path.cwd()
vendor = root / 'vendor'
checksum_files = sorted(vendor.glob('*/.cargo-checksum.json'))
if not checksum_files:
    print('no vendored Cargo checksum metadata found', file=sys.stderr)
    raise SystemExit(1)

failures = []
for checksum_file in checksum_files:
    data = json.loads(checksum_file.read_text(encoding='utf-8'))
    package_dir = checksum_file.parent
    for rel, expected in data.get('files', {}).items():
        path = package_dir / rel
        if not path.is_file():
            failures.append(f'missing vendored file: {path.relative_to(root)}')
            continue
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != expected:
            failures.append(
                f'vendor checksum mismatch: {path.relative_to(root)} expected={expected} actual={actual}'
            )

if failures:
    print('\n'.join(failures), file=sys.stderr)
    raise SystemExit(1)
PY

# Snapshot manifest is deliberately repository-wide. On a Git checkout, also
# verify that it matches the exact staged Git bytes used by GitHub/CI. Source
# archives have no index, so they still verify directly by SHA-256.
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  python3 scripts/update-repository-manifest.py --check
fi
sha256sum -c --quiet REPOSITORY_MANIFEST.sha256

echo 'repository layout + include/vendor/manifest integrity: PASS'
