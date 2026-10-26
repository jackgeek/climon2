#!/bin/bash
# Builds climon2 from its locked dependencies.
#
#   ./build.sh            release build into climon2/target/release
#   ./build.sh --debug    debug build into climon2/target/debug
#   ./build.sh -- ARGS    extra arguments passed to cargo build
#
# Run the acceptance tests against the result with CLIMON2_BIN_DIR set to the printed directory.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
profile=release
cargo_args=()
while [ $# -gt 0 ]; do
  case "$1" in
    --debug) profile=debug ;;
    --release) profile=release ;;
    --) shift; cargo_args+=("$@"); break ;;
    -h|--help) sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done

command -v cargo >/dev/null || { echo "cargo not found; install Rust from https://rustup.rs" >&2; exit 1; }

[ "$profile" = release ] && cargo_args=(--release "${cargo_args[@]+"${cargo_args[@]}"}")
cargo build --manifest-path "$ROOT/climon2/Cargo.toml" --locked "${cargo_args[@]+"${cargo_args[@]}"}"

echo "Built climon2 into $ROOT/climon2/target/$profile"
