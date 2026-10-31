#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
export CARGO_BUILD_JOBS=1
printf 'Throwaway #52 offline fixture; no native terminal interaction\n'
rustc --version
printf 'Formatting\n'
cargo fmt --check
printf 'Tests: locked dependencies, one build job and one test thread\n'
cargo test --locked --quiet -j 1 -- --test-threads=1
printf 'Clippy\n'
cargo clippy --locked --quiet -j 1 --all-targets -- -D warnings
printf 'PASS: fixture/model checks only; Terminal.app/iTerm2/VS Code remain UNTESTED\n'
