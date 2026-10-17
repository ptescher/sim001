#!/bin/sh
# Wraps the release sim in a macOS app bundle that declares itself a game, which is what lets
# macOS turn on Game Mode for it (it also needs the sim to be full screen: --fullscreen or F11).
# Game Mode can't be asked for from code. Usage:
#   cargo build --release && tools/macos-app.sh && open "target/release/SIM001.app" --args --fullscreen
set -e
cd "$(dirname "$0")/.."
bin=target/release/sim001
[ -x "$bin" ] || { echo "build the sim first: cargo build --release" >&2; exit 1; }
app="target/release/SIM001.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$bin" "$app/Contents/MacOS/sim001"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key><string>SIM001</string>
	<key>CFBundleDisplayName</key><string>SIM001</string>
	<key>CFBundleIdentifier</key><string>dev.t1s.sim</string>
	<key>CFBundleExecutable</key><string>sim001</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleVersion</key><string>1</string>
	<key>CFBundleShortVersionString</key><string>0.1.0</string>
	<key>LSApplicationCategoryType</key><string>public.app-category.games</string>
	<key>LSSupportsGameMode</key><true/>
	<key>GCSupportsGameMode</key><true/>
	<key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Ad-hoc signed, so macOS treats the copy as a normal app.
codesign --force --sign - "$app" >/dev/null 2>&1 || true
echo "$app"
