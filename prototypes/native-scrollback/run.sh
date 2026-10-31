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
printf 'Offline native-observer unit tests (no Apple events or TTY access)\n'
python3 -m unittest -q test_terminal_app test_interactive_input test_terminal_gui test_capacity test_paste_input test_rendition
printf 'PASS: offline checks only; these do not execute or qualify native hosts\n'
