#!/usr/bin/env bash
# THROWAWAY #56/#43: actual-engine API probes, not native-host qualification.
# Run from anywhere: bash prototypes/alacritty-forwarding/run.sh
set -euo pipefail
cd "$(dirname "$0")"

revision=bbb2cbf9470a6707b1205fbddbdb25109b14e2fd
source_directory=.cache/alacritty
if [[ ! -d "$source_directory" ]]; then
    mkdir -p "$source_directory"
    git -C "$source_directory" init --quiet
    git -C "$source_directory" fetch --quiet --depth 1 \
        https://github.com/alacritty/alacritty.git "$revision"
    git -C "$source_directory" checkout --quiet --detach FETCH_HEAD
fi
[[ "$(git -C "$source_directory" rev-parse HEAD)" == "$revision" ]] || {
    echo "FAIL: unexpected Alacritty revision" >&2
    exit 1
}
[[ -z "$(git -C "$source_directory" status --porcelain)" ]] || {
    echo "FAIL: refusing modified upstream source" >&2
    exit 1
}
[[ "$(rustc --version | cut -d ' ' -f 2)" == "1.96.0" ]] || {
    echo "FAIL: select pinned Rust 1.96.0" >&2
    exit 1
}

export CARGO_TARGET_DIR="$PWD/.cache/target"
echo "Alacritty: $revision (unchanged)"
rustc --version
python3 check_surface.py
cargo fmt --manifest-path probe/Cargo.toml -- --check
cargo clippy -j1 --quiet --locked --manifest-path probe/Cargo.toml --all-targets -- -D warnings
cargo test -j1 --quiet --locked --manifest-path probe/Cargo.toml -- --test-threads=1
if command -v node >/dev/null && [[ -d host-probe/node_modules ]]; then
    cargo build -j1 --quiet --locked --manifest-path probe/Cargo.toml --example derive_osc8
    for uri in 'http://a/ok' $'http://a/\xc2\x9c\xc2\x9b2J'; do
        printf 'xterm.js replay of %q: ' "$uri"
        "$CARGO_TARGET_DIR/debug/examples/derive_osc8" "$uri" | node host-probe/replay.mjs | tail -1
    done
else
    echo "SKIP: xterm.js host replay (run: npm ci --prefix host-probe)"
fi
echo "PASS: exercised engine/codec seams; production input/delivery and native hosts remain UNPROVEN"
