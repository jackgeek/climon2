#!/bin/sh
# Usage: lock-crates.sh <project-dir>
# Resolves /work/<project-dir>/Cargo.toml against crates.io and writes /out/Cargo.lock.
# Existing lock entries are kept. Runs no build code.
set -eu
cp -R "/work/$1" /tmp/project
cd /tmp/project
cargo metadata --format-version 1 >/dev/null
cp Cargo.lock /out/Cargo.lock
