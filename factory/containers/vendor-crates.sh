#!/bin/sh
# Usage: vendor-crates.sh <subject-dir>
# Downloads the locked dependencies of /work/<subject-dir> into /vendor. Runs no build code.
set -eu
cd "/work/$1"
cargo vendor --locked --versioned-dirs /vendor/crates >/vendor/cargo-config.toml
touch /vendor/.complete
