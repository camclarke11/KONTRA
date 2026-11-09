#!/usr/bin/env bash
# Builds this fork's standalone player as a normal Mac app, /Applications/KONTRA.app,
# so Spotlight, Launchpad and the Dock can find it. Re-run after pulling updates.
#
#   tools/mac-app/build-app.sh               build, bundle and install to /Applications
#   tools/mac-app/build-app.sh --no-install  build and bundle into target/mac-app/ only
#
# Transcribing recordings needs its model installed once (tools/transcribe-model).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/../.." && pwd)"
out="$src/target/mac-app"
app="$out/KONTRA.app"
dest="/Applications/KONTRA.app"
bundle_id="io.github.camclarke11.kontra"
lsregister="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

install=1
case "${1:-}" in
  "") ;;
  --no-install) install=0 ;;
  *) echo "usage: $0 [--no-install]" >&2; exit 2 ;;
esac

[[ -f "$src/Cargo.toml" ]] || { echo "KONTRA source not found at $src" >&2; exit 1; }
# rustup puts cargo here; a fresh shell may not have it on PATH yet.
# shellcheck disable=SC1091
[[ -f "$HOME/.cargo/env" ]] && . "$HOME/.cargo/env"

echo "==> Building the standalone player (release)"
(cd "$src" && cargo build --locked --release -p kontra-window)

# The full version (3.0.0-cam.1), and its numbers alone for macOS (3.0.0).
full="$(sed -nE 's/^version = "([^"]+)"/\1/p' "$src/plugin/Cargo.toml" | head -1)"
version="${full%%-*}"
build="$(git -C "$src" rev-list --count HEAD)"
commit="$(git -C "$src" rev-parse --short HEAD)"

echo "==> Assembling $app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$src/target/release/kontra-window" "$app/Contents/MacOS/KONTRA"

icns="$out/KONTRA.icns"
if [[ ! -f "$icns" || "$here/make-icon.swift" -nt "$icns" ]]; then
  echo "==> Drawing the icon"
  iconset="$out/KONTRA.iconset"
  rm -rf "$iconset"
  mkdir -p "$iconset"
  swift "$here/make-icon.swift" "$out/icon-1024.png"
  for s in 16 32 128 256 512; do
    sips -z "$s" "$s" "$out/icon-1024.png" --out "$iconset/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) "$out/icon-1024.png" --out "$iconset/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$iconset" -o "$icns"
fi
cp "$icns" "$app/Contents/Resources/KONTRA.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>KONTRA</string>
  <key>CFBundleDisplayName</key><string>KONTRA</string>
  <key>CFBundleIdentifier</key><string>${bundle_id}</string>
  <key>CFBundleExecutable</key><string>KONTRA</string>
  <key>CFBundleIconFile</key><string>KONTRA</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${build}</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>KONTRA only uses audio input if you turn it on in its Settings menu.</string>
  <key>NSHumanReadableCopyright</key><string>KONTRA by DerpcatMusic and contributors, Apache-2.0. This is camclarke11's fork, version ${full}, built from ${commit}.</string>
  <key>KONTRASourceCommit</key><string>${commit}</string>
</dict>
</plist>
PLIST
plutil -lint -s "$app/Contents/Info.plist"

echo "==> Signing (ad-hoc: runs on this Mac, not for sharing)"
codesign --force --sign - "$app"
codesign --verify --strict "$app"

if (( install )); then
  if [[ -e "$dest" ]]; then
    existing="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$dest/Contents/Info.plist" 2>/dev/null || true)"
    if [[ "$existing" != "$bundle_id" ]]; then
      echo "$dest exists but isn't this build (bundle id: ${existing:-unknown}); leaving it alone" >&2
      exit 1
    fi
    if pgrep -f "$dest/Contents/MacOS/KONTRA" >/dev/null; then
      echo "note: KONTRA is running; quit and reopen it to use the new build"
    fi
    rm -rf "$dest"
  fi
  echo "==> Installing $dest"
  ditto "$app" "$dest"
  "$lsregister" -f "$dest"
  mdimport "$dest"
  echo "Done: KONTRA ${full} (${commit}). Find it with Spotlight (⌘ Space, \"KONTRA\")."
else
  echo "Done: $app"
fi
