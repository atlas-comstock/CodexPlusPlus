#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-0.0.0}"
ARCH="${2:-$(uname -m)}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DIST="$ROOT/dist/freecodex"
STAGE="$DIST/stage"
BINARY_DIR="${BINARY_DIR:-$ROOT/target/release}"
DMG="$DIST/FreeCodex-${VERSION}-macos-${ARCH}.dmg"
ICON_SOURCE="$ROOT/assets/images/codex-plus-plus.png"
ICON_NAME="freecodex.icns"
ICON_ICNS="$DIST/$ICON_NAME"
README="$ROOT/scripts/installer/macos/FreeCodex-使用说明.txt"

rm -rf "$DIST"
mkdir -p "$STAGE"

prepare_icon() {
  local iconset="$DIST/freecodex.iconset"
  rm -rf "$iconset"
  mkdir -p "$iconset"

  sips -z 16 16 "$ICON_SOURCE" --out "$iconset/icon_16x16.png" >/dev/null
  sips -z 32 32 "$ICON_SOURCE" --out "$iconset/icon_16x16@2x.png" >/dev/null
  sips -z 32 32 "$ICON_SOURCE" --out "$iconset/icon_32x32.png" >/dev/null
  sips -z 64 64 "$ICON_SOURCE" --out "$iconset/icon_32x32@2x.png" >/dev/null
  sips -z 128 128 "$ICON_SOURCE" --out "$iconset/icon_128x128.png" >/dev/null
  sips -z 256 256 "$ICON_SOURCE" --out "$iconset/icon_128x128@2x.png" >/dev/null
  sips -z 256 256 "$ICON_SOURCE" --out "$iconset/icon_256x256.png" >/dev/null
  sips -z 512 512 "$ICON_SOURCE" --out "$iconset/icon_256x256@2x.png" >/dev/null
  sips -z 512 512 "$ICON_SOURCE" --out "$iconset/icon_512x512.png" >/dev/null
  sips -z 1024 1024 "$ICON_SOURCE" --out "$iconset/icon_512x512@2x.png" >/dev/null

  iconutil -c icns "$iconset" -o "$ICON_ICNS"
}

create_app() {
  local app_name="$1"
  local executable_name="$2"
  local binary_path="$3"
  local bundle_id="$4"
  local app_dir="$STAGE/$app_name.app"

  if [ ! -x "$binary_path" ]; then
    echo "error: binary not found or not executable: $binary_path" >&2
    return 1
  fi

  rm -rf "$app_dir"
  mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Resources"
  cp "$binary_path" "$app_dir/Contents/MacOS/$executable_name"
  cp "$ICON_ICNS" "$app_dir/Contents/Resources/$ICON_NAME"
  chmod +x "$app_dir/Contents/MacOS/$executable_name"
  printf 'APPL????' > "$app_dir/Contents/PkgInfo"
  cat > "$app_dir/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>$app_name</string>
  <key>CFBundleDisplayName</key>
  <string>$app_name</string>
  <key>CFBundleIdentifier</key>
  <string>$bundle_id</string>
  <key>CFBundleVersion</key>
  <string>$VERSION</string>
  <key>CFBundleShortVersionString</key>
  <string>$VERSION</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleSignature</key>
  <string>????</string>
  <key>CFBundleExecutable</key>
  <string>$executable_name</string>
  <key>CFBundleIconFile</key>
  <string>$ICON_NAME</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>LSUIElement</key>
  <true/>
</dict>
</plist>
PLIST
}

sign_app() {
  local app_dir="$1"
  local executable
  executable="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app_dir/Contents/Info.plist")"
  codesign --force --sign - "$app_dir/Contents/MacOS/$executable"
  codesign --force --sign - "$app_dir"
}

verify_app() {
  local app_dir="$1"
  plutil -lint "$app_dir/Contents/Info.plist" >/dev/null
  codesign -dv "$app_dir" >/dev/null 2>&1
}

prepare_icon
create_app "FreeCodex" "FreeCodex" "$BINARY_DIR/codex-plus-plus" "com.freecodex.launcher"
ln -s /Applications "$STAGE/Applications"
cp "$README" "$STAGE/使用说明.txt"

sign_app "$STAGE/FreeCodex.app"
verify_app "$STAGE/FreeCodex.app"

hdiutil create -volname "FreeCodex" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
echo "$DMG"