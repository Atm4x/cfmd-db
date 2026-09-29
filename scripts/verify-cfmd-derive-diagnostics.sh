#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
TMP="$ROOT/target/cfmd-derive-ui"
rm -rf "$TMP"
mkdir -p "$TMP/src"
cat > "$TMP/Cargo.toml" <<TOML
[package]
name = "cfmd-derive-ui"
version = "0.0.0"
edition = "2024"

[dependencies]
cfmd = { path = "$ROOT/crates/cfmd" }

[workspace]
TOML

run_fail() {
  local expected="$1"
  cat > "$TMP/src/main.rs"
  local log="$TMP/stderr.log"
  if cargo check --manifest-path "$TMP/Cargo.toml" --offline >"$TMP/stdout.log" 2>"$log"; then
    echo "derive UI case unexpectedly compiled" >&2
    cat "$TMP/src/main.rs" >&2
    exit 1
  fi
  if ! grep -Fq "$expected" "$log"; then
    echo "derive UI case did not emit expected diagnostic: $expected" >&2
    cat "$log" >&2
    exit 1
  fi
}

run_fail 'CfmdEntity requires #[cfmd(key = "application.stable-key")]' <<'RS'
use cfmd::{CfmdEntity, Id};

#[derive(CfmdEntity)]
struct MissingKey {
    #[cfmd(id)]
    id: Id<MissingKey>,
}

fn main() {}
RS

run_fail '#[cfmd(id)] field must have type Id<BadIdentity>' <<'RS'
use cfmd::CfmdEntity;

#[derive(CfmdEntity)]
#[cfmd(key = "ui.bad-id")]
struct BadIdentity {
    #[cfmd(id)]
    id: u64,
}

fn main() {}
RS

run_fail 'CfmdEntity requires exactly one #[cfmd(id)] field; found 0' <<'RS'
use cfmd::CfmdEntity;

#[derive(CfmdEntity)]
#[cfmd(key = "ui.no-id")]
struct NoIdentity {
    value: i64,
}

fn main() {}
RS

run_fail 'many(...) requires target = <Type>' <<'RS'
use cfmd::{CfmdEntity, Id};

#[derive(CfmdEntity)]
#[cfmd(key = "ui.bad-many")]
#[cfmd(many(name = children, via = parent))]
struct BadMany {
    #[cfmd(id)]
    id: Id<BadMany>,
}

fn main() {}
RS

run_fail 'reverse-many accessor `children` conflicts with stored field `children`' <<'RS'
use cfmd::{CfmdEntity, Id};

struct Child;

#[derive(CfmdEntity)]
#[cfmd(key = "ui.shadow-many")]
#[cfmd(many(name = children, target = Child, via = parent))]
struct ShadowMany {
    #[cfmd(id)]
    id: Id<ShadowMany>,
    children: i64,
}

fn main() {}
RS

run_fail 'duplicate reverse-many accessor `children`' <<'RS'
use cfmd::{CfmdEntity, Id};

struct Child;

#[derive(CfmdEntity)]
#[cfmd(key = "ui.duplicate-many")]
#[cfmd(many(name = children, target = Child, via = parent))]
#[cfmd(many(name = children, target = Child, via = owner))]
struct DuplicateMany {
    #[cfmd(id)]
    id: Id<DuplicateMany>,
}

fn main() {}
RS

run_fail '#[cfmd(via = ...)] is only valid on Many<T> virtual relationship fields' <<'RS'
use cfmd::{CfmdEntity, Id};

#[derive(CfmdEntity)]
#[cfmd(key = "ui.bad-via")]
struct BadVia {
    #[cfmd(id)]
    id: Id<BadVia>,
    #[cfmd(via = parent)]
    value: i64,
}

fn main() {}
RS

rm -rf "$TMP"
echo 'cfmd derive compile-time diagnostics: PASS'
