#!/usr/bin/env bash
# Builds Mnemo.app (universal: Apple Silicon + Intel) and a .dmg. Run on a Mac with Xcode Command Line Tools
# (`xcode-select --install`) and Rust (https://rustup.rs).
set -euo pipefail
cd "$(dirname "$0")/.."
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
APP=dist/Mnemo.app
rm -rf "$APP" && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/mnemo" \
  target/aarch64-apple-darwin/release/mnemo target/x86_64-apple-darwin/release/mnemo
cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
# icon
ICONSET=$(mktemp -d)/mnemo.iconset && mkdir -p "$ICONSET"
for s in 16 32 64 128 256 512; do
  sips -z $s $s assets/icons/mnemo.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s*2)); [ $d -le 1024 ] && sips -z $d $d assets/icons/mnemo.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/mnemo.icns"
# ad-hoc signature (lets Apple Silicon run it; Gatekeeper still blocks the first launch: see README, « Premier lancement »)
codesign --force --deep --sign - "$APP"
hdiutil create -volname Mnemo -srcfolder "$APP" -ov -format UDZO dist/Mnemo-macos.dmg
echo "OK : $APP et dist/Mnemo-macos.dmg"
