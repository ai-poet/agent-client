#!/bin/sh
# CheapRouter macOS installer.
#
#   curl -fsSL https://s3.cheaprouter.cc/cheaprouter-releases/install-mac.sh | sh
#
# Downloads the latest release zip with curl — which, unlike a browser, sets
# no com.apple.quarantine attribute — so the ad-hoc-signed app opens without
# Gatekeeper prompts. Later updates arrive in-app through the signed
# CheapRouter update feed (the 0.3.x client used Sparkle for the first hop to
# this one); the updater verifies the signature and clears quarantine itself.
set -eu

FEED="https://s3.cheaprouter.cc/cheaprouter-releases/appcast.xml"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "This installer is for macOS." >&2
  exit 1
fi

arch=$(uname -m)
feed=$(curl -fsSL "$FEED") || {
  echo "Could not fetch $FEED" >&2
  exit 1
}

pick() {
  printf '%s' "$feed" | grep -o "$1" | head -1 | cut -d'"' -f2
}

case "$arch" in
  x86_64)
    url=$(pick 'url="[^"]*-x86_64\.zip"')
    if [ -z "$url" ]; then
      # The appcast only lists the arm64 build. The Intel zip ships next to it
      # as <name>-x86_64.zip, so probe that before falling back.
      arm=$(pick 'url="[^"]*\.zip"')
      case "$arm" in
        *.zip)
          candidate="${arm%.zip}-x86_64.zip"
          if curl -fsI "$candidate" >/dev/null 2>&1; then
            url=$candidate
          else
            url=$arm
          fi
          ;;
      esac
    fi
    ;;
  *)
    # Apple silicon builds carry no architecture suffix in their file name.
    url=$(printf '%s' "$feed" \
      | grep -o 'url="[^"]*\.zip"' \
      | grep -v 'x86_64\.zip' \
      | head -1 | cut -d'"' -f2)
    [ -n "$url" ] || url=$(pick 'url="[^"]*\.zip"')
    ;;
esac
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

binary=$(find "$app/Contents/MacOS" -maxdepth 1 -type f -perm -u+x 2>/dev/null | head -1)
if [ -n "$binary" ] && command -v lipo >/dev/null 2>&1; then
  binarchs=$(lipo -archs "$binary" 2>/dev/null || true)
  case " $binarchs " in
    *" $arch "*) : ;;
    *) echo "warning: this build does not include $arch; it may need Rosetta." >&2 ;;
  esac
fi

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
