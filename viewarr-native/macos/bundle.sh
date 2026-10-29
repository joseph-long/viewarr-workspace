#!/bin/sh
# Build viewarr.app from this crate.
#
#   macos/bundle.sh              universal (Apple Silicon + Intel) bundle
#   macos/bundle.sh --host-only  just this Mac's architecture, for a quick try
#
# The bundle lands in target/bundle/viewarr.app. VERSION in the environment
# sets the version it reports; otherwise Cargo.toml's is used.
#
# There is no Developer ID to sign with, so the bundle is ad-hoc signed: enough
# for Apple Silicon to run it and for the signature to cover Info.plist, but
# Gatekeeper still treats a downloaded copy as unidentified (see README).
set -eu

cd "$(dirname "$0")/.."

host_only=false
case "${1:-}" in
    --host-only) host_only=true ;;
    "") ;;
    *) echo "usage: $0 [--host-only]" >&2; exit 2 ;;
esac

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

plutil -lint "$app/Contents/Info.plist" >/dev/null
codesign --force --sign - "$app"
codesign --verify --strict "$app"

echo "$app ($version, $(lipo -archs "$app/Contents/MacOS/viewarr"))"
