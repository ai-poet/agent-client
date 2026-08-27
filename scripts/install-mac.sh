#!/bin/sh
# CheapRouter macOS installer.
#
#   curl -fsSL https://s3.cheaprouter.cc/cheaprouter-releases/install-mac.sh | sh
#
# Downloads the latest release zip with curl — which, unlike a browser, sets
# no com.apple.quarantine attribute — so the ad-hoc-signed app opens without
# Gatekeeper prompts. Later updates arrive in-app via Sparkle, which verifies
# our EdDSA signature and clears quarantine itself.
set -eu

FEED="https://s3.cheaprouter.cc/cheaprouter-releases/appcast.xml"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "This installer is for macOS." >&2
  exit 1
fi

url=$(curl -fsSL "$FEED" | grep -o 'url="[^"]*\.zip"' | head -1 | cut -d'"' -f2)
if [ -z "$url" ]; then
  echo "Could not resolve the latest release from $FEED" >&2
  exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading ${url##*/} ..."
curl -fL --progress-bar "$url" -o "$tmp/app.zip"

# ditto preserves the bundle's structure and extended attributes the way
# Archive Utility would.
ditto -xk "$tmp/app.zip" "$tmp/extract"
app=$(find "$tmp/extract" -maxdepth 2 -name "*.app" -print | head -1)
if [ -z "$app" ]; then
  echo "The archive did not contain an app bundle." >&2
  exit 1
fi
name=$(basename "$app")

# /Applications is admin-writable; fall back to ~/Applications otherwise.
destination="/Applications"
if [ ! -w "$destination" ]; then
  destination="$HOME/Applications"
  mkdir -p "$destination"
fi

rm -rf "${destination:?}/$name"
ditto "$app" "$destination/$name"
# curl leaves no quarantine attribute; clear defensively anyway so a copied
# or re-downloaded archive behaves the same.
xattr -dr com.apple.quarantine "$destination/$name" 2>/dev/null || true

echo "Installed $destination/$name"
open "$destination/$name"
