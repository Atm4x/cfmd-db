#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if grep -Eq '^kernel-[A-Za-z0-9_-]+[[:space:]]*=' crates/cfmd/Cargo.toml; then
  echo 'public cfmd crate must not depend on kernel-* crates' >&2
  exit 1
fi

python3 - <<'PYAPI'
from pathlib import Path
import tomllib

manifest = tomllib.loads(Path("crates/cfmd/Cargo.toml").read_text(encoding="utf-8"))
dependencies = set(manifest.get("dependencies", {}))
if dependencies != {"cfmd-runtime", "cfmd-derive"}:
    raise SystemExit(
        "public cfmd crate may depend only on cfmd-runtime and cfmd-derive, "
        f"got {sorted(dependencies)}"
    )
PYAPI

if rg -n '\bkernel_[A-Za-z0-9_]+' crates/cfmd/src crates/cfmd/tests crates/cfmd/examples >/dev/null; then
  echo 'public cfmd source leaked a kernel crate path' >&2
  exit 1
fi

if rg -n '\b(cfmd_runtime|cfmd_derive)\b' crates/cfmd/tests crates/cfmd/examples >/dev/null; then
  echo 'public facade consumer tests/examples must import only cfmd, not implementation crates' >&2
  exit 1
fi

cargo check -p cfmd --all-targets --locked --offline
cargo test -p cfmd --all-targets --locked --offline

echo 'public Rust facade boundary: PASS'
