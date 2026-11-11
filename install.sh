#!/bin/bash
set -euo pipefail

VERSION='0.1.0'
BASE_URL=${CLIMON2_RELEASE_BASE_URL:-https://github.com/jackgeek/climon2/releases/download}

fail() {
    printf 'climon2-install.sh: error: %s\n' "$*" >&2
    exit 1
}

case "$(uname -s):$(uname -m)" in
    Darwin:arm64|Darwin:x86_64) platform=macos-universal ;;
    Linux:x86_64)
        getconf GNU_LIBC_VERSION >/dev/null 2>&1 || fail 'Linux requires GNU libc'
        platform=linux-x86_64-gnu
        ;;
    *) fail 'supported platforms: macOS arm64/x86_64 and GNU/Linux x86_64' ;;
esac
NAME="climon2-$VERSION-$platform"
ARCHIVE="$NAME.tar.gz"

workdir=$(mktemp -d "${TMPDIR:-/tmp}/climon2-install.XXXXXX") || fail 'cannot create temporary directory'
trap 'rm -rf -- "$workdir"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
cd "$workdir" || fail 'cannot enter temporary directory'

curl --fail --location --silent --show-error \
    "$BASE_URL/v$VERSION/$ARCHIVE" -o "$ARCHIVE" || fail 'archive download failed'
curl --fail --location --silent --show-error \
    "$BASE_URL/v$VERSION/SHA256SUMS" -o SHA256SUMS || fail 'checksum download failed'
awk -v file="$ARCHIVE" '$2 == file { print }' SHA256SUMS > archive.sha256
[ "$(wc -l < archive.sha256)" -eq 1 ] || fail 'archive checksum must occur exactly once'
if [ "$platform" = linux-x86_64-gnu ]; then
    sha256sum -c archive.sha256 || fail 'archive checksum verification failed'
else
    shasum -a 256 -c archive.sha256 || fail 'archive checksum verification failed'
fi
tar -xzf "$ARCHIVE" || fail 'archive extraction failed'
[ -d "$NAME" ] || fail 'archive version directory is missing'
if [ "$platform" = macos-universal ] && command -v xattr >/dev/null 2>&1; then
    if ! xattr -dr com.apple.quarantine "$NAME" 2>/dev/null; then
        printf 'climon2-install.sh: warning: could not remove quarantine; macOS may require manual removal\n' >&2
    fi
fi
cd "$NAME" || fail 'cannot enter release directory'
./install "$@" || fail 'installer failed'
