#!/usr/bin/env bash
# Packages the Windows Desktop (x86_64 / arm64): Inno Setup installer,
# portable zip and MyGo second-hop .tar.gz update archive. Does not require
# NSIS. Run from the desktop module root:
#
#   cmd/sailry-desktop/windows/package.sh [-arch <amd64|arm64>] [-artifacts <dir>] [--require-update-key] [--dry-run]
#
# -artifacts is the CI native job's directory of the Windows iroh, Turso,
# capture and app-local Microsoft C++ runtime; without it, .runtime/native
# libraries already built on the host are used.
#
# .runtime/package/out/windows-<arch> receives the application directory, the
# Inno Setup installer CheapRouter-<version>-<rust_arch>-Setup.exe, the
# portable archive cheaprouter-<version>-<triple>.zip, and the second-hop MyGo
# update archive cheaprouter-<version>-windows-<arch>.tar.gz.
#
# Env:
#   MYGO_UPDATE_FEED_URL, MYGO_UPDATE_PUBLIC_KEY: MyGo second-hop updater
#     feed base URL and base64 Ed25519 public key
#   WINDOWS_CERTIFICATE, WINDOWS_CERTIFICATE_PASSWORD: optional base64 .pfx
#     Authenticode certificate and password (Windows only)
#   SAILRY_REVISION, SAILRY_SENTRY_DSN, SAILRY_TELEMETRY_KEY: optional
#     telemetry build values
set -euo pipefail

test -f go.mod && test -f cmd/sailry-desktop/mygo.json || { echo 'run from the desktop module root' >&2; exit 1; }

artifacts=()
arch_input=""
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
      test $# -ge 2 || { echo '-arch needs amd64/x86_64 or arm64/aarch64' >&2; exit 1; }
      arch_input="$2"
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
      echo 'usage: package.sh [-arch <amd64|arm64>] [-artifacts <dir>] [--require-update-key] [--dry-run]' >&2
      exit 1
      ;;
  esac
done

if test -z "$arch_input"; then
  case "${SAILRY_ARCH:-${PROCESSOR_ARCHITECTURE:-$(uname -m)}}" in
    ARM64|arm64|aarch64) arch_input=arm64 ;;
    *) arch_input=amd64 ;;
  esac
fi

case "$arch_input" in
  amd64|x86_64|x64)
    arch=amd64
    rust_arch=x86_64
    triple=x86_64-pc-windows-msvc
    ;;
  arm64|aarch64)
    arch=arm64
    rust_arch=aarch64
    triple=aarch64-pc-windows-msvc
    ;;
  *)
    echo "unsupported Windows architecture: $arch_input" >&2
    exit 1
    ;;
esac

unset GOROOT
export GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0

platform=windows/$arch
out=.runtime/package/out/windows-$arch
resources=cmd/sailry-desktop/resources/windows-$arch
iss_script=cmd/sailry-desktop/windows/cheaprouter.iss
test -f "$iss_script" || { echo "missing Inno Setup script: $iss_script" >&2; exit 1; }

app_name=$(jq -r .name cmd/sailry-desktop/mygo.json)
app_slug=$(jq -r .slug cmd/sailry-desktop/mygo.json)
app_identifier=$(jq -r .identifier cmd/sailry-desktop/mygo.json)
version=$(jq -r .version cmd/sailry-desktop/mygo.json)
# MyGo embeds the top-level PNG in the exe; only this script reads windows.icon.
app_icon_rel=$(jq -r '.icon // ""' cmd/sailry-desktop/mygo.json)
win_icon_rel=$(jq -r '.windows.icon // ""' cmd/sailry-desktop/mygo.json)

resolve_mygo_path() {
  local raw="$1"
  if test -z "$raw"; then
    return 1
  fi
  case "$raw" in
    /*) printf '%s\n' "$raw" ;;
    *) printf '%s\n' "cmd/sailry-desktop/$raw" ;;
  esac
}

app_icon=$(resolve_mygo_path "$app_icon_rel") || { echo 'mygo.json icon is required' >&2; exit 1; }
win_icon=$(resolve_mygo_path "$win_icon_rel") || { echo 'mygo.json windows.icon is required' >&2; exit 1; }
case "$win_icon" in
  *.[iI][cC][oO]) ;;
  *) echo 'mygo.json windows.icon must point to an .ico file' >&2; exit 1 ;;
esac
test -f "$app_icon" || { echo "missing app icon configured in mygo.json: $app_icon" >&2; exit 1; }
test -f "$win_icon" || { echo "missing windows.icon configured in mygo.json: $win_icon" >&2; exit 1; }

installer=$out/$app_name-$version-$rust_arch-Setup.exe
portable=$out/$app_slug-$version-$triple.zip
mygo_archive=$out/$app_slug-$version-windows-$arch.tar.gz

revision=${SAILRY_REVISION:-$(git rev-parse HEAD)}
telemetry=github.com/ai-poet/agent-client/internal/node/telemetry
update_pkg=github.com/ai-poet/agent-client/internal/update
flags="-X github.com/ai-poet/agent-client/internal/core.Default=~/.sailry"
flags="$flags -X $telemetry.BuildRevision=$revision"
test -z "${SAILRY_SENTRY_DSN:-}" || flags="$flags -X $telemetry.BuildDSN=$SAILRY_SENTRY_DSN"
test -z "${SAILRY_TELEMETRY_KEY:-}" || flags="$flags -X $telemetry.BuildKey=$SAILRY_TELEMETRY_KEY"

update_feed_base="${MYGO_UPDATE_FEED_URL:-$(jq -r '.updates.url // ""' cmd/sailry-desktop/mygo.json)}"
update_feed_manifest="${update_feed_base%/}/update-windows-$arch.json"
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
  grep -q 'AppId={{7DC6C35B-FA40-4A95-B37A-626BF64556C5}' "$iss_script" ||
    { echo "Inno Setup AppId mismatch in $iss_script" >&2; exit 1; }
  grep -q '^SetupIconFile=' "$iss_script" ||
    { echo "missing SetupIconFile in $iss_script" >&2; exit 1; }
  grep -q '^UninstallDisplayIcon=' "$iss_script" ||
    { echo "missing UninstallDisplayIcon in $iss_script" >&2; exit 1; }
  grep -q 'AppUserModelID:' "$iss_script" ||
    { echo "missing shortcut AppUserModelID in $iss_script" >&2; exit 1; }
  cat <<EOF
[dry-run] Windows package plan:
  platform          : $platform ($triple, rust_arch=$rust_arch)
  app               : $app_name ($app_slug)
  version           : $version
  app_user_model_id : $app_identifier
  app_icon          : $app_icon
  windows_icon      : $win_icon
  installer_engine  : Inno Setup ($iss_script)
  mygo_feed_url     : $update_feed_manifest
  mygo_public_key   : ${update_public_key:-(unconfigured)}
  outputs:
    installer       : $installer
    portable_zip    : $portable
    mygo_archive    : $mygo_archive
EOF
  exit 0
fi

find_iscc() {
  if command -v ISCC.exe >/dev/null 2>&1; then
    command -v ISCC.exe
    return 0
  fi
  if command -v iscc >/dev/null 2>&1; then
    command -v iscc
    return 0
  fi
  for base in "${PROGRAMFILES:-}" "${PROGRAMFILES_X86:-}" "/c/Program Files (x86)" "/c/Program Files"; do
    test -n "$base" || continue
    if test -f "$base/Inno Setup 6/ISCC.exe"; then
      printf '%s\n' "$base/Inno Setup 6/ISCC.exe"
      return 0
    fi
  done
  return 1
}

find_signtool() {
  local kits_root="/c/Program Files (x86)/Windows Kits/10/bin"
  test -d "$kits_root" || return 1
  local search_arches="x64 x86"
  if test "$arch" = arm64; then
    search_arches="arm64 x64 x86"
  fi
  local tool_arch dir
  for tool_arch in $search_arches; do
    while IFS= read -r dir; do
      if test -f "$dir/$tool_arch/signtool.exe"; then
        printf '%s\n' "$dir/$tool_arch/signtool.exe"
        return 0
      fi
    done < <(find "$kits_root" -maxdepth 1 -mindepth 1 -type d | sort -Vr)
    if test -f "$kits_root/$tool_arch/signtool.exe"; then
      printf '%s\n' "$kits_root/$tool_arch/signtool.exe"
      return 0
    fi
  done
  return 1
}

to_win_path() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -w "$1"
  else
    printf '%s\n' "$1"
  fi
}

iscc=$(find_iscc) || { echo 'ISCC.exe (Inno Setup 6) is required' >&2; exit 1; }

mygo_config_backup=""
stage_tmp=""
cert_file=""
cleanup() {
  if test -n "$mygo_config_backup" && test -f "$mygo_config_backup"; then
    mv "$mygo_config_backup" cmd/sailry-desktop/mygo.json
  fi
  if test -n "$stage_tmp" && test -d "$stage_tmp"; then
    rm -rf -- "$stage_tmp"
  fi
  if test -n "$cert_file" && test -f "$cert_file"; then
    rm -f -- "$cert_file"
  fi
}
trap cleanup EXIT

go run ./native/fetch -target "$platform" ${artifacts[@]+"${artifacts[@]}"} -bundle "$resources"
go build -o .runtime/package/bin/mygo github.com/egoist/mygo/cmd/mygo

rm -rf -- "$resources/licenses"
bash internal/node/host/deployment/licenses.sh "$platform" "$triple" "$resources/licenses" ./cmd/sailry-desktop
dirty=false
if test -n "$(git status --porcelain)"; then dirty=true; fi
jq -n --arg revision "$revision" --argjson dirty "$dirty" --arg target "$triple" --arg version "$version" \
  '{version:1,source_revision:$revision,source_dirty:$dirty,profile:"release",target:$target,application_version:$version}' \
  > "$resources/build.json"
go run ./native/fetch -target "$platform" -seal "$resources"

mygo_config_backup=$(mktemp "${TMPDIR:-/tmp}/mygo-config-XXXXXX")
cp cmd/sailry-desktop/mygo.json "$mygo_config_backup"
if test -n "$update_public_key"; then
  jq --arg url "${update_feed_base%/}" --arg key "$update_public_key" \
    '.updates.url = $url | .updates.publicKey = $key' "$mygo_config_backup" > cmd/sailry-desktop/mygo.json
else
  echo 'warning: MYGO_UPDATE_PUBLIC_KEY is not set; MyGo updater will build unconfigured' >&2
  jq 'del(.updates)' "$mygo_config_backup" > cmd/sailry-desktop/mygo.json
fi

rm -rf -- "$out"
build_status=0
# -skip-installer leaves NSIS out completely; we build the first-hop Inno Setup
# installer below. Unset MYGO_UPDATER_PRIVATE_KEY so mygo build does not write
# a premature archive before optional Authenticode signing.
env -u MYGO_UPDATER_PRIVATE_KEY MYGO_LDFLAGS="$flags" \
  .runtime/package/bin/mygo build -platform "$platform" -skip-installer cmd/sailry-desktop || build_status=$?
mv "$mygo_config_backup" cmd/sailry-desktop/mygo.json
mygo_config_backup=""
test "$build_status" -eq 0

for name in "$app_name.exe" native-seal.json build.json; do
  test -f "$out/$name" || { echo "the package has no $name" >&2; exit 1; }
done

# Carry license and attribution notices alongside the application.
if test -f LICENSE; then
  cp LICENSE "$out/LICENSE"
fi
if test -f NOTICE; then
  cp NOTICE "$out/NOTICE"
fi

# Optional Authenticode signing of shipped executables before bundling.
signtool=""
if test -n "${WINDOWS_CERTIFICATE:-}" && test -n "${WINDOWS_CERTIFICATE_PASSWORD:-}"; then
  signtool=$(find_signtool) || { echo 'signtool.exe is required when WINDOWS_CERTIFICATE is set' >&2; exit 1; }
  cert_file=$(mktemp "${TMPDIR:-/tmp}/cheaprouter-cert-XXXXXX.pfx")
  printf '%s' "$WINDOWS_CERTIFICATE" | base64 -d > "$cert_file"
  cert_win=$(to_win_path "$cert_file")
  for exe_file in "$out/$app_name.exe" "$out/helpers/agent-desktop.exe"; do
    if test -f "$exe_file"; then
      "$signtool" sign /f "$cert_win" /p "$WINDOWS_CERTIFICATE_PASSWORD" /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 "$(to_win_path "$exe_file")"
    fi
  done
fi

# Stage a clean copy of the app tree so portable zip, Inno Setup and MyGo tar.gz
# all package the exact same files without pulling in installer/zip outputs.
stage_tmp=$(mktemp -d "${TMPDIR:-/tmp}/cheaprouter-win-stage-XXXXXX")
app_stage="$stage_tmp/$app_slug-$version-$triple"
mkdir -p "$app_stage"
(cd "$out" && find . -mindepth 1 -maxdepth 1 ! -name '*.zip' ! -name '*-Setup.exe' ! -name '*.tar.gz' -exec cp -R {} "$app_stage/" \;)

# 1. Portable zip (matches scripts/bundle-windows.ts naming and top-folder layout).
portable_abs="$(cd "$out" && pwd)/$app_slug-$version-$triple.zip"
rm -f -- "$portable_abs"
if command -v tar >/dev/null 2>&1 && tar --version 2>&1 | grep -qi bsdtar; then
  tar -a -c -f "$(to_win_path "$portable_abs")" -C "$(to_win_path "$stage_tmp")" "$app_slug-$version-$triple"
elif command -v zip >/dev/null 2>&1; then
  (cd "$stage_tmp" && zip -q -r -X "$portable_abs" "$app_slug-$version-$triple")
else
  pwsh -NoProfile -Command "Compress-Archive -Path '$(to_win_path "$app_stage")' -DestinationPath '$(to_win_path "$portable_abs")' -Force"
fi
test -f "$portable" || { echo "failed to produce portable archive $portable" >&2; exit 1; }

# 2. First-hop Inno Setup installer (compatible with legacy /SILENT /DIR= updater).
icon_file="$win_icon"
license_file="LICENSE"
"$iscc" \
  "/DAppVersion=$version" \
  "/DArch=$rust_arch" \
  "/DStageDir=$(to_win_path "$app_stage")" \
  "/DOutputDir=$(to_win_path "$(cd "$out" && pwd)")" \
  "/DIconFile=$(to_win_path "$(cd "$(dirname "$icon_file")" && pwd)/$(basename "$icon_file")")" \
  "/DLicenseFile=$(to_win_path "$(cd "$(dirname "$license_file")" && pwd)/$(basename "$license_file")")" \
  "/DAppUserModelID=$app_identifier" \
  "$(to_win_path "$(pwd)/$iss_script")"
test -f "$installer" || { echo "ISCC did not produce $installer" >&2; exit 1; }

if test -n "$signtool" && test -n "$cert_file"; then
  "$signtool" sign /f "$(to_win_path "$cert_file")" /p "$WINDOWS_CERTIFICATE_PASSWORD" /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 "$(to_win_path "$installer")"
fi

# 3. Second-hop MyGo update archive (.tar.gz containing the app directory contents).
go run ./cmd/mygo-manifest pack -root "$app_stage" -out "$mygo_archive" -target "windows-$arch"

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$installer" "$portable" "$mygo_archive"
else
  shasum -a 256 "$installer" "$portable" "$mygo_archive"
fi
