#!/bin/bash
# Package a configure-built macOS engine as a standalone app.
set -euo pipefail

project_dir="$(cd "$(dirname "$0")/.." && pwd)"
build_dir="$(cd "${1:?Usage: macbuild.tool build-dir output.zip}" && pwd)"
output="${2:?Usage: macbuild.tool build-dir output.zip}"
mkdir -p "$(dirname "$output")"
output="$(cd "$(dirname "$output")" && pwd)/$(basename "$output")"
package_dir="$(mktemp -d "$build_dir/package.XXXXXX")"
trap 'rm -rf "$package_dir"' EXIT

app="$package_dir/onscripter-new.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$build_dir/onscripter-new" "$app/Contents/MacOS/onscripter-new"
chmod 755 "$app/Contents/MacOS/onscripter-new"
cp "$project_dir/Resources/Bundle/Info-mac.plist" "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"

iconset="$package_dir/AppIcon.iconset"
mkdir "$iconset"
for size in 16 32 128 256 512; do
  cp "$project_dir/Resources/Bundle/Images.xcassets/AppIcon-mac.appiconset/${size}x${size}.png" \
    "$iconset/icon_${size}x${size}.png"
done
for size in 16 32 128 256; do
  double=$((size * 2))
  cp "$project_dir/Resources/Bundle/Images.xcassets/AppIcon-mac.appiconset/${double}x${double}.png" \
    "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/onscripter-new.icns"
rm -r "$iconset"

# The release must run without Homebrew or the CI checkout.
otool -L "$app/Contents/MacOS/onscripter-new"
if otool -L "$app/Contents/MacOS/onscripter-new" | tail -n +2 | \
    awk '{print $1}' | grep -Ev '^(/System/Library/|/usr/lib/)'; then
  echo 'Unexpected non-system dynamic library dependency' >&2
  exit 1
fi
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
"$app/Contents/MacOS/onscripter-new" --version
cp "$project_dir/LICENSE" "$project_dir/LICENSE-BSD" "$project_dir/LICENSE-GPLv2" "$package_dir/"
cat > "$package_dir/INSTALL.txt" <<'EOF'
onscripter-new 1.8 - macOS

Requires macOS 14 or newer. Use the arm64 download for Apple Silicon or the
x86_64 download for Intel. Back up your game folder and saves, then put
onscripter-new.app in your existing Umineko Project folder and open it.

For the shared script and menu updates, extract en.file, wh.file, ru.file,
and graphics from onscripter-new-android-assets.zip into the same game folder.
The app also supports game files inside Contents/Resources.

The app is ad-hoc signed and is not notarized. If macOS blocks opening it,
allow this downloaded app in System Settings > Privacy & Security.

Existing iCloud saves require enable-icloud in ons.cfg. Local saves are used
by default. This package requires an existing compatible game installation.
EOF
ditto -c -k --sequesterRsrc "$package_dir" "$output"
echo "Packaged $output"
