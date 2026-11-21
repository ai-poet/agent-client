#!/bin/sh
# Run the desktop from source, on macOS, Linux and Windows (Git Bash/MSYS).
# Usage: ./dev-desktop.sh [dev|release|<profile>] [extra desktop flags]
#
# dev      $HOME/.sailry-dev   (default; keeps release data untouched)
# release  $HOME/.sailry
# <name>   .runtime/profiles/<name>
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$root"

case "$(uname -s)" in
    Darwin) platform=darwin ;;
    Linux) platform=linux ;;
    # Git Bash and MSYS report MINGW*/MSYS*; the native scripts map the same way.
    MINGW*|MSYS*) platform=windows ;;
    *) printf '%s\n' "dev-desktop: unsupported platform $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
    arm64|aarch64) architecture=arm64 ;;
    x86_64|amd64) architecture=amd64 ;;
    *) printf '%s\n' "dev-desktop: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac

native="$root/.runtime/native/$platform-$architecture"
case "$platform" in
    darwin) iroh=libsailry_iroh.dylib; turso=libturso_sync_sdk_kit.dylib ;;
    windows) iroh=sailry_iroh.dll; turso=turso_sync_sdk_kit.dll ;;
    *) iroh=libsailry_iroh.so; turso=libturso_sync_sdk_kit.so ;;
esac

# Both libraries are dlopen'd at runtime; build them once, then reuse.
if [ ! -f "$native/$iroh" ]; then
    printf '%s\n' "dev-desktop: building $iroh (first run, a few minutes)"
    sh native/iroh/build.sh
fi
if [ ! -f "$native/$turso" ]; then
    printf '%s\n' "dev-desktop: building $turso (first run, a few minutes)"
    sh native/turso/build.sh
fi

profile=${1:-dev}
if [ "$#" -gt 0 ]; then
    shift
fi
case "$profile" in
    dev) data="$HOME/.sailry-dev" ;;
    release) data="$HOME/.sailry" ;;
    *) data="$root/.runtime/profiles/$profile" ;;
esac

# Git Bash hands POSIX paths to Windows programs, which do not resolve them:
# hand the loader and the app native paths where cygpath can produce them.
if [ "$platform" = windows ] && command -v cygpath >/dev/null 2>&1; then
    native_env=$(cygpath -w "$native")
    data=$(cygpath -w "$data")
else
    native_env=$native
fi

export GOTOOLCHAIN=go1.27.3
export CGO_ENABLED=0
export SAILRY_IROH_LIB="$native_env/$iroh"
export SAILRY_TURSO_LIB="$native_env/$turso"
if [ -d "$native/helpers" ]; then
    export SAILRY_HELPERS_DIR="$native_env/helpers"
fi

exec go run ./cmd/sailry-desktop --data-dir "$data" "$@"
