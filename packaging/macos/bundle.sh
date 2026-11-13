#!/bin/sh
# Builds AnyQuake.app and AnyQuake-<version>.dmg from an already built binary.
#
#   packaging/macos/bundle.sh <anyquake binary> <output dir>
#
# The binary may be a universal one (see .github/workflows/release.yml).
# Signing and the disk image need macOS (codesign, hdiutil); elsewhere only
# the .app directory is created.
set -eu

binary=$1
out=$2
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)

app="$out/AnyQuake.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
sed "s/@VERSION@/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
cp "$binary" "$app/Contents/MacOS/anyquake"
chmod 755 "$app/Contents/MacOS/anyquake"
cp "$here/AnyQuake.icns" "$app/Contents/Resources/"
printf 'APPL????' > "$app/Contents/PkgInfo"
echo "created $app"

if command -v codesign >/dev/null; then
    # Ad-hoc signature by default. Set CODESIGN_IDENTITY to a "Developer ID
    # Application" identity to sign for distribution (and notarization).
    if [ -n "${CODESIGN_IDENTITY:-}" ]; then
        codesign --force --options runtime --timestamp \
            --sign "$CODESIGN_IDENTITY" "$app"
    else
        codesign --force --sign - "$app"
    fi
    codesign --verify --strict "$app"
fi

if command -v hdiutil >/dev/null; then
    staging=$(mktemp -d)
    cp -R "$app" "$staging/"
    ln -s /Applications "$staging/Applications"
    dmg="$out/AnyQuake-$version.dmg"
    rm -f "$dmg"
    hdiutil create -volname AnyQuake -srcfolder "$staging" -fs HFS+ \
        -format UDZO -ov "$dmg"
    rm -rf "$staging"
    echo "created $dmg"
fi
