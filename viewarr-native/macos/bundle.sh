#!/bin/sh
# Build viewarr.app from this crate.
#
#   macos/bundle.sh              universal (Apple Silicon + Intel) bundle
#   macos/bundle.sh --host-only  just this Mac's architecture, for a quick try
#   macos/bundle.sh --install    also replace /Applications/viewarr.app with it
#
# The flags combine. The bundle lands in target/bundle/viewarr.app. VERSION in the environment
# sets the version it reports; otherwise Cargo.toml's is used.
#
# There is no Developer ID to sign with, so the bundle is ad-hoc signed: enough
# for Apple Silicon to run it and for the signature to cover Info.plist, but
# Gatekeeper still treats a downloaded copy as unidentified (see README).
set -eu

cd "$(dirname "$0")/.."

host_only=false
install=false
for arg in "$@"; do
    case "$arg" in
        --host-only) host_only=true ;;
        --install) install=true ;;
        *) echo "usage: $0 [--host-only] [--install]" >&2; exit 2 ;;
    esac
done

version="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)}"

# Matches LSMinimumSystemVersion in Info.plist. Without it each architecture
# gets rustc's own default floor, and the two differ.
export MACOSX_DEPLOYMENT_TARGET=11.0

if $host_only; then
    cargo build --release
    binaries="target/release/viewarr"
else
    binaries=""
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
        cargo build --release --target "$target"
        binaries="$binaries target/$target/release/viewarr"
    done
fi

app=target/bundle/viewarr.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

# shellcheck disable=SC2086 # $binaries is a space-separated list on purpose
lipo -create -output "$app/Contents/MacOS/viewarr" $binaries
sed "s/@VERSION@/$version/g" macos/Info.plist > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
# The binary carries a few hundred crates' worth of code whose licenses ask
# for their notices to travel with it.
cp LICENSE THIRD_PARTY_LICENSES.txt "$app/Contents/Resources/"

plutil -lint "$app/Contents/Info.plist" >/dev/null
codesign --force --sign - "$app"
codesign --verify --strict "$app"

echo "$app ($version, $(lipo -archs "$app/Contents/MacOS/viewarr"))"

if $install; then
    dest=/Applications/viewarr.app
    # Swapping the bundle out from under a running copy leaves that copy
    # executing a deleted binary, so ask it to quit first and wait until it
    # has. Matched by bundle id, so a `cargo run` copy is left alone, and
    # asked only if running: telling a closed app to quit launches it.
    app_id=$(plutil -extract CFBundleIdentifier raw "$app/Contents/Info.plist")
    running() {
        [ "$(osascript -e "application id \"$app_id\" is running")" = true ]
    }
    if running; then
        osascript -e "tell application id \"$app_id\" to quit"
        tries=50
        while running; do
            tries=$((tries - 1))
            if [ "$tries" -le 0 ]; then
                echo "viewarr did not quit; close it and rerun with --install" >&2
                exit 1
            fi
            sleep 0.2
        done
    fi
    rm -rf "$dest"
    ditto "$app" "$dest"
    echo "installed $dest"
fi
