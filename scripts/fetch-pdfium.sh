#!/usr/bin/env bash
# Fetch a prebuilt libpdfium.so from the bblanchon/pdfium-binaries releases.
#
# Output: vendor/pdfium/lib/libpdfium.so
#
# pdfium is not packaged by Arch or Debian, so we vendor a known-good build.
# The release tag below is the version we're pinning against; bump explicitly
# (don't track "latest") so dev/CI/install builds stay byte-identical.

set -euo pipefail

# Pdfium release tag from https://github.com/bblanchon/pdfium-binaries/releases
PDFIUM_RELEASE="${PDFIUM_RELEASE:-chromium/7543}"

case "$(uname -m)" in
    x86_64) ARCH="x64" ;;
    aarch64) ARCH="arm64" ;;
    *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$REPO_ROOT/vendor/pdfium"
URL="https://github.com/bblanchon/pdfium-binaries/releases/download/${PDFIUM_RELEASE}/pdfium-linux-${ARCH}.tgz"

if [ -f "$DEST/lib/libpdfium.so" ]; then
    echo "✓ libpdfium.so already present at $DEST/lib/libpdfium.so"
    exit 0
fi

echo "Fetching pdfium ($PDFIUM_RELEASE) from $URL..."
mkdir -p "$DEST"
TMP_TARBALL="$(mktemp --suffix=.tgz)"
trap 'rm -f "$TMP_TARBALL"' EXIT

curl -L --fail --show-error --output "$TMP_TARBALL" "$URL"
tar -xzf "$TMP_TARBALL" -C "$DEST"

if [ ! -f "$DEST/lib/libpdfium.so" ]; then
    echo "✗ Expected libpdfium.so not found at $DEST/lib/libpdfium.so" >&2
    ls -la "$DEST" >&2
    exit 1
fi

echo "✓ libpdfium.so installed at $DEST/lib/libpdfium.so"
