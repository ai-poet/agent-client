#!/usr/bin/env bash
# Packages the macOS Desktop for release: build, sign, seal, notarize, disk
# image, first-hop Sparkle zip and second-hop MyGo update archive. Run from
# the desktop module root on the target architecture:
#
#   cmd/sailry-desktop/macos/package.sh [-artifacts <dir>] [-arch <arm64|amd64>] [--require-update-key] [--dry-run]
#
# -artifacts is the CI native job's directory of iroh, Turso and capture libraries;
# without it the libraries already built in .runtime/native are used.
#
# .runtime/package/out/darwin-<arch> receives CheapRouter.app, the disk image
# CheapRouter-<triple>.dmg, CheapRouter-<triple>-unsigned.zip (for Sparkle),
# and cheaprouter-<version>-darwin-<arch>.tar.gz (for MyGo second-hop updates).
#
# The environment selects signing, with the names the release settings use:
#
#   SAILRY_SIGNING=ad-hoc (default) or developer-id
#   developer-id: APPLE_SIGNING_IDENTITY (or WAKU_SIGNING_IDENTITY),
#     APPLE_TEAM_ID and, for a temporary keychain, APPLE_SIGNING_KEYCHAIN
#   SAILRY_NOTARIZE=1 (developer-id only): APPLE_NOTARY_PROFILE, or
#     APPLE_API_KEY_PATH, APPLE_API_KEY_ID, APPLE_API_ISSUER_ID
#   MYGO_UPDATE_FEED_URL, MYGO_UPDATE_PUBLIC_KEY: MyGo second-hop updater
#     feed base URL and base64 Ed25519 public key
#   SAILRY_REVISION, SAILRY_SENTRY_DSN, SAILRY_TELEMETRY_KEY: telemetry build
#     values, each optional
#
# The package opens the release profile ~/.sailry. Nothing here runs it.
set -euo pipefail

test -f go.mod && test -f cmd/sailry-desktop/mygo.json || { echo 'run from the desktop module root' >&2; exit 1; }

artifacts=()
arch_override=""
dry_run=0
require_update_key=${MYGO_REQUIRE_UPDATE_KEY:-0}

while test $# -gt 0; do
  case "$1" in
    -artifacts)
      test $# -ge 2 || { echo '-artifacts needs a directory' >&2; exit 1; }
      test -d "$2" || { echo '-artifacts needs a directory' >&2; exit 1; }
      artifacts=(-artifacts "$2")
      shift 2
      ;;
    -arch|--arch)
      test $# -ge 2 || { echo '-arch needs arm64 or amd64' >&2; exit 1; }
      arch_override="$2"
      shift 2
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    --require-update-key)
      require_update_key=1
      shift
      ;;
    *)
      echo 'usage: package.sh [-artifacts <dir>] [-arch <arm64|amd64>] [--require-update-key] [--dry-run]' >&2
      exit 1
      ;;
  esac
done

if test "$dry_run" -eq 0; then
  test "$(uname -s)" = Darwin || { echo 'package.sh needs macOS (pass --dry-run to validate on other platforms)' >&2; exit 1; }
fi

signing=${SAILRY_SIGNING:-ad-hoc}
notarize=${SAILRY_NOTARIZE:-0}
case "$signing" in
  ad-hoc)
    test "$notarize" != 1 || { echo 'SAILRY_NOTARIZE needs SAILRY_SIGNING=developer-id' >&2; exit 1; }
    identity=-
    ;;
  developer-id)
    signing_identity="${APPLE_SIGNING_IDENTITY:-${WAKU_SIGNING_IDENTITY:-}}"
    test -n "$signing_identity" && test -n "${APPLE_TEAM_ID:-}" ||
      { echo 'APPLE_SIGNING_IDENTITY (or WAKU_SIGNING_IDENTITY) and APPLE_TEAM_ID are required' >&2; exit 1; }
    identity=$signing_identity
    ;;
  *) echo 'SAILRY_SIGNING must be ad-hoc or developer-id' >&2; exit 1 ;;
esac

credentials=()
if test "$notarize" = 1; then
  if test -n "${APPLE_NOTARY_PROFILE:-}"; then
    credentials=(--keychain-profile "$APPLE_NOTARY_PROFILE")
  elif test -f "${APPLE_API_KEY_PATH:-}" && test -n "${APPLE_API_KEY_ID:-}" && test -n "${APPLE_API_ISSUER_ID:-}"; then
    credentials=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID")
  else
    echo 'APPLE_NOTARY_PROFILE or APPLE_API_KEY_PATH, APPLE_API_KEY_ID and APPLE_API_ISSUER_ID are required' >&2
    exit 1
  fi
fi

unset GOROOT
export GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0

raw_arch="${arch_override:-$(uname -m)}"
case "$raw_arch" in
  arm64|aarch64) arch=arm64 triple=aarch64-apple-darwin ;;
  x86_64|amd64) arch=amd64 triple=x86_64-apple-darwin ;;
  *) echo "unsupported architecture: $raw_arch" >&2; exit 1 ;;
esac

platform=darwin/$arch
out=.runtime/package/out/darwin-$arch
app_name=$(jq -r .name cmd/sailry-desktop/mygo.json)
app_slug=$(jq -r .slug cmd/sailry-desktop/mygo.json)
bundle_identifier=$(jq -r .identifier cmd/sailry-desktop/mygo.json)
version=$(jq -r .version cmd/sailry-desktop/mygo.json)
sparkle_public_key=$(jq -r '.macos.infoPlist.SUPublicEDKey // ""' cmd/sailry-desktop/mygo.json)
disk_image_identifier="${bundle_identifier}.disk-image"
app=$out/$app_name.app
image=$out/$app_name-$triple.dmg
update=$out/$app_name-$triple-unsigned.zip
mygo_archive=$out/$app_slug-$version-darwin-$arch.tar.gz
resources=cmd/sailry-desktop/resources/darwin-$arch

bundle_version=${version%%[-+]*}
IFS=. read -r version_major version_minor version_patch <<< "$bundle_version"
bundle_build=$((10#$version_major * 1000000 + 10#$version_minor * 1000 + 10#$version_patch))

revision=${SAILRY_REVISION:-$(git rev-parse HEAD)}
telemetry=github.com/ai-poet/cheaprouter-harness/internal/node/telemetry
update_pkg=github.com/ai-poet/cheaprouter-harness/internal/update
flags="-X github.com/ai-poet/cheaprouter-harness/internal/core.Default=~/.sailry"
flags="$flags -X $telemetry.BuildRevision=$revision"
test -z "${SAILRY_SENTRY_DSN:-}" || flags="$flags -X $telemetry.BuildDSN=$SAILRY_SENTRY_DSN"
test -z "${SAILRY_TELEMETRY_KEY:-}" || flags="$flags -X $telemetry.BuildKey=$SAILRY_TELEMETRY_KEY"

update_feed_base="${MYGO_UPDATE_FEED_URL:-$(jq -r '.updates.url // ""' cmd/sailry-desktop/mygo.json)}"
update_feed_manifest="${update_feed_base%/}/update-darwin-$arch.json"
update_public_key="${MYGO_UPDATE_PUBLIC_KEY:-$(jq -r '.updates.publicKey // ""' cmd/sailry-desktop/mygo.json)}"
if test "$require_update_key" = 1 && test -z "$update_public_key"; then
  echo 'MYGO_UPDATE_PUBLIC_KEY is required for release packaging' >&2
  exit 1
fi
if test -n "$update_feed_base"; then
  flags="$flags -X $update_pkg.BuildFeedURL=$update_feed_manifest"
fi
if test -n "$update_public_key"; then
  flags="$flags -X $update_pkg.BuildPublicKey=$update_public_key"
  flags="$flags -X github.com/egoist/mygo.packageUpdateFeed=$update_feed_manifest"
  flags="$flags -X github.com/egoist/mygo.packageUpdateKey=$update_public_key"
fi

if test "$dry_run" -eq 1; then
  test "$bundle_identifier" = "sh.waku" || { echo "unexpected bundle identifier: $bundle_identifier" >&2; exit 1; }
  test -n "$sparkle_public_key" || { echo 'mygo.json macos.infoPlist.SUPublicEDKey is required for Sparkle appcast signing' >&2; exit 1; }
  cat <<EOF
[dry-run] macOS package plan:
  platform          : $platform ($triple)
  app               : $app_name ($bundle_identifier)
  version           : $version (CFBundleShortVersionString=$bundle_version, CFBundleVersion=$bundle_build)
  signing           : $signing (notarize=$notarize)
  sparkle_pub_key   : $sparkle_public_key
  mygo_feed_url     : $update_feed_manifest
  mygo_public_key   : ${update_public_key:-(unconfigured)}
  outputs:
    bundle          : $app
    dmg             : $image
    sparkle_zip     : $update
    mygo_archive    : $mygo_archive
EOF
  exit 0
fi

# Restore both temporary keychain search list and mygo.json on exit.
keychain_previous=()
keychain_modified=0
mygo_config_backup=""
cleanup() {
  if test -n "$mygo_config_backup" && test -f "$mygo_config_backup"; then
    mv "$mygo_config_backup" cmd/sailry-desktop/mygo.json
  fi
  if test "$keychain_modified" -eq 1; then
    security list-keychains -d user -s ${keychain_previous[@]+"${keychain_previous[@]}"}
  fi
}
trap cleanup EXIT

if test -n "${APPLE_SIGNING_KEYCHAIN:-}"; then
  while IFS= read -r line; do
    line=${line#"${line%%[![:space:]]*}"}
    line=${line#\"}
    keychain_previous+=("${line%\"}")
  done < <(security list-keychains -d user)
  keychain_modified=1
  security list-keychains -d user -s "$APPLE_SIGNING_KEYCHAIN" ${keychain_previous[@]+"${keychain_previous[@]}"}
fi

go run ./native/fetch -target "$platform" ${artifacts[@]+"${artifacts[@]}"} -bundle "$resources"
# Pinned MyGo reaches the helper app's main executable before its sibling.
# Intel's unsigned sibling must be signed before codesign validates that app.
helper_signing=(--force --sign "$identity" --preserve-metadata=entitlements)
if test "$identity" != -; then
  helper_signing+=(--options runtime --timestamp)
fi
codesign "${helper_signing[@]}" "$resources/helpers/Sailry Computer Use.app/Contents/MacOS/agent-desktop-macos-helper"
go build -o .runtime/package/bin/mygo github.com/egoist/mygo/cmd/mygo

# Shipped beside the runtimes: the dependency notices and the build record
# the updater requires of every package it installs.
rm -rf -- "$resources/licenses"
bash internal/node/host/deployment/licenses.sh "$platform" "$triple" "$resources/licenses" ./cmd/sailry-desktop
dirty=false
if test -n "$(git status --porcelain)"; then dirty=true; fi
jq -n --arg revision "$revision" --argjson dirty "$dirty" --arg target "$triple" --arg version "$version" \
  '{version:1,source_revision:$revision,source_dirty:$dirty,profile:"release",target:$target,application_version:$version}' \
  > "$resources/build.json"

mygo_config_backup=$(mktemp "${TMPDIR:-/tmp}/mygo-config-XXXXXX")
cp cmd/sailry-desktop/mygo.json "$mygo_config_backup"
if test -n "$update_public_key"; then
  jq --arg url "${update_feed_base%/}" --arg key "$update_public_key" \
    '.updates.url = $url | .updates.publicKey = $key' "$mygo_config_backup" > cmd/sailry-desktop/mygo.json
else
  echo 'warning: MYGO_UPDATE_PUBLIC_KEY is not set; MyGo updater will build unconfigured' >&2
  jq 'del(.updates)' "$mygo_config_backup" > cmd/sailry-desktop/mygo.json
fi

rm -rf -- "$app" "$image" "$update" "$mygo_archive"
# mygo signs nested code and the bundle. Unset MYGO_UPDATER_PRIVATE_KEY here
# so mygo build does not emit a premature unsealed/unnotarized update archive;
# we pack the finalized bundle with cmd/mygo-manifest after seal + notarize.
build_status=0
env -u MYGO_UPDATER_PRIVATE_KEY MYGO_LDFLAGS="$flags" \
  .runtime/package/bin/mygo build -platform "$platform" -sign "$identity" -skip-notarize -skip-dmg cmd/sailry-desktop || build_status=$?
mv "$mygo_config_backup" cmd/sailry-desktop/mygo.json
mygo_config_backup=""
test "$build_status" -eq 0
test -d "$app"

# Align bundle identity, numeric CFBundleVersion, and SUPublicEDKey with main
# (sh.waku, CheapRouter, major*1_000_000 + minor*1_000 + patch) so Sparkle
# signs and performs an in-place upgrade instead of rejecting or duplicating.
/usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $bundle_identifier" "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleExecutable $app_name" "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $bundle_version" "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $bundle_build" "$app/Contents/Info.plist"
if test -n "$sparkle_public_key"; then
  /usr/libexec/PlistBuddy -c "Set :SUPublicEDKey $sparkle_public_key" "$app/Contents/Info.plist" 2>/dev/null || \
    /usr/libexec/PlistBuddy -c "Add :SUPublicEDKey string $sparkle_public_key" "$app/Contents/Info.plist"
fi

# Signing rewrote the native libraries and plist changes updated the bundle:
# record the libraries as shipped and sign the bundle again over the records.
go run ./native/fetch -target "$platform" -seal "$app" -sign "$identity"

codesign --verify --deep --strict "$app"
if test "$signing" = developer-id; then
  find "$app" -type f \( -perm -u+x -o -name '*.dylib' -o -name '*.so' \) -print0 |
    while IFS= read -r -d '' file; do
      file -b "$file" | grep -q 'Mach-O' || continue
      details=$(codesign --display --verbose=4 "$file" 2>&1)
      case "$details" in
        *"TeamIdentifier=$APPLE_TEAM_ID"*) ;;
        *) echo "unexpected signing team: $file" >&2; exit 1 ;;
      esac
      case "$details" in
        *'(runtime)'*Timestamp=* | *Timestamp=*'(runtime)'*) ;;
        *) echo "missing hardened runtime or secure timestamp: $file" >&2; exit 1 ;;
      esac
    done
fi

submit() {
  result=$(xcrun notarytool submit "$1" "${credentials[@]}" --wait --timeout 45m --output-format json)
  case "$result" in
    *'"status":"Accepted"'* | *'"status": "Accepted"'*) ;;
    *) echo "notarization did not succeed: $result" >&2; exit 1 ;;
  esac
}

if test "$notarize" = 1; then
  archive=$out/$app_name-notarize.zip
  rm -f -- "$archive"
  ditto -c -k --keepParent "$app" "$archive"
  submit "$archive"
  rm -f -- "$archive"
  xcrun stapler staple "$app"
  xcrun stapler validate "$app"
  codesign --verify --deep --strict "$app"
  spctl --assess --type execute --verbose=2 "$app"
fi

stage=$(mktemp -d "$out/.dmg-XXXXXX")
cp -R "$app" "$stage/$app_name.app"
ln -s /Applications "$stage/Applications"
hdiutil create -quiet -volname "$app_name" -srcfolder "$stage" -fs HFS+ -format UDZO "$image"
rm -rf -- "$stage"
if test "$signing" = developer-id; then
  codesign --sign "$identity" --timestamp --identifier "$disk_image_identifier" "$image"
  codesign --verify --strict "$image"
fi
hdiutil verify -quiet "$image"
if test "$notarize" = 1; then
  submit "$image"
  xcrun stapler staple "$image"
  xcrun stapler validate "$image"
  spctl --assess --type open --context context:primary-signature "$image"
fi

# First-hop Sparkle update archive (ditto preserves symlinks and signatures).
ditto -c -k --norsrc --keepParent "$app" "$update"

# Second-hop MyGo update archive built from the finalized sealed/notarized bundle.
go run ./cmd/mygo-manifest pack -root "$out" -out "$mygo_archive" -target "darwin-$arch" "$app_name.app"

shasum -a 256 "$image" "$update" "$mygo_archive"
