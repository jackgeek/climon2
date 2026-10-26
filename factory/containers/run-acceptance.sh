#!/bin/sh
# Usage: run-acceptance.sh <subject-dir> <pytest args...>
# Builds the Cargo project at /work/<subject-dir> offline, then runs pytest from /work.
# Exit 97 means the build failed; its log tail is printed.
set -u
subject="$1"
shift
cd "/work/$subject" || { echo "no such subject: $subject"; exit 97; }
if [ -s /vendor/cargo-config.toml ]; then
  cargo build --release --locked --offline --config /vendor/cargo-config.toml >/out/build.log 2>&1
else
  cargo build --release --locked --offline >/out/build.log 2>&1
fi
if [ $? -ne 0 ]; then
  tail -c 20000 /out/build.log
  exit 97
fi
cd /work
CLIMON2_BIN_DIR=/build/target/release exec python3 -m pytest -p no:cacheprovider \
  --rootdir=/work --junitxml=/out/report.xml "$@"
