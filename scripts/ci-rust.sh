#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_NET_OFFLINE=true
bash ./scripts/verify-repository.sh
bash ./scripts/verify-public-api.sh
bash ./scripts/verify-cfmd-derive-diagnostics.sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --all-targets --locked --offline
