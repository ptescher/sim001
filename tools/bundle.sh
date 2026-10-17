#!/bin/bash
# Bundles the launcher, the sim, the ground station, ArduPlane SITL, the maps and the docs into one
# app. Build it on the OS you want it for.
#   tools/bundle.sh [--no-build] [--no-docs]
#     macOS    target/bundle/SIM001.app, and SIM001-macos.zip beside it
#     Linux    target/bundle/sim001-linux/ (run sim001_launcher), and sim001-linux.tar.gz
#     Windows  (Git Bash) target/bundle/sim001-windows/ and sim001-windows.zip; untested
# In the bundle the programs sit beside the launcher, and the rest (maps, docs, ArduPilot's files,
# the console's terrain map) in the resources: Contents/Resources of the .app, or the folder itself.
# The launcher tells the programs where, with SIM001_HOME, SIM001_BIN and SIM001_DATA (controls.cfg
# and flight logs go to the user's data folder, since the app itself is read-only).
# ArduPlane SITL is bundled if ardupilot/build_native.sh has built it, and both the sim and the
# station then run without Docker (the station starts one ArduPlane per plane, yours and the
# computer pilots'). Without it they fall back to Docker.
set -e
cd "$(dirname "$0")/.."
ROOT=$PWD
build=1; docs=1
for a in "$@"; do case $a in --no-build) build=0;; --no-docs) docs=0;; esac; done

case "$(uname -s)" in
  Darwin) os=macos; exe="";;
  Linux) os=linux; exe="";;
  MINGW*|MSYS*|CYGWIN*) os=windows; exe=".exe";;
  *) echo "unsupported OS" >&2; exit 1;;
esac

if [ $build = 1 ]; then
  echo "== building the sim, the console and the launcher"
  cargo build --release
  (cd domains/teleoperation/console && cargo build --release)
  (cd domains/teleoperation/launcher && cargo build --release)
fi
SIM=target/release/sim001$exe
GCS=domains/teleoperation/console/target/release/sim001_gcs$exe
LAUNCHER=domains/teleoperation/launcher/target/release/sim001_launcher$exe
for f in "$SIM" "$GCS" "$LAUNCHER"; do [ -x "$f" ] || { echo "missing $f (build first)" >&2; exit 1; }; done

# The console's terrain map of the Alpine Valley comes from the sim.
if [ ! -f domains/teleoperation/console/assets/terrain_map.png ]; then
  echo "== exporting the terrain map"
  "$SIM" --export-map domains/teleoperation/console/assets
fi
if [ $docs = 1 ]; then
  echo "== rendering the docs from the saved test results"
  cargo run -q --release -p expectation_cli -- docs expectations docs || echo "(docs not re-rendered: using docs/ as it is)"
fi

OUT=target/bundle
rm -rf "$OUT" 2>/dev/null || { sleep 2; rm -rf "$OUT"; }; mkdir -p "$OUT"
case $os in
  macos) APP="$OUT/SIM001.app"; BIN="$APP/Contents/MacOS"; RES="$APP/Contents/Resources";;
  *) APP="$OUT/sim001-$os"; BIN="$APP"; RES="$APP";;
esac
mkdir -p "$BIN" "$RES"

cp "$LAUNCHER" "$BIN/"; cp "$SIM" "$BIN/"; cp "$GCS" "$BIN/"
if [ -x .ardupilot/build/sitl/bin/arduplane$exe ]; then
  cp .ardupilot/build/sitl/bin/arduplane$exe "$BIN/"
  mkdir -p "$RES/.ardupilot/Tools/autotest/models"
  cp .ardupilot/Tools/autotest/models/plane.parm "$RES/.ardupilot/Tools/autotest/models/"
else
  echo "note: no native ArduPlane (ardupilot/build_native.sh): the bundle will need Docker for ArduPilot"
fi

echo "== resources: maps, ArduPilot's files, docs, terrain map"
mkdir -p "$RES/maps"
for m in maps/*/; do
  n=$(basename "$m")
  mkdir -p "$RES/maps/$n"
  cp "$m"map.txt "$RES/maps/$n/" 2>/dev/null || true
  # The built pack, without the download cache.
  [ -d "$m/build" ] && rsync -a --exclude cache "$m/build" "$RES/maps/$n/"
done
cp maps/README.md "$RES/maps/" 2>/dev/null || true
mkdir -p "$RES/ardupilot"; cp ardupilot/* "$RES/ardupilot/"
rsync -a --exclude results docs/ "$RES/docs/"
rsync -a domains/teleoperation/console/assets/ "$RES/assets/"
cp controls.cfg.example "$RES/"
cp README.md "$RES/"
echo "SIM001 $(git describe --always --dirty 2>/dev/null || echo dev)" > "$RES/sim001-bundle"

case $os in
  macos)
    cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key><string>SIM001</string>
	<key>CFBundleDisplayName</key><string>SIM001</string>
	<key>CFBundleIdentifier</key><string>dev.t1s.sim.launcher</string>
	<key>CFBundleExecutable</key><string>sim001_launcher</string>
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
    # Ad-hoc signed, so that macOS treats the copy as a normal app (right-click, Open the first time
    # if it came from the internet). A Developer ID signature and notarization are up to you.
    codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || echo "(codesign failed: the app is unsigned)"
    (cd "$OUT" && ditto -c -k --keepParent SIM001.app SIM001-macos.zip)
    echo "$APP  ($(du -sh "$APP" | cut -f1)), $OUT/SIM001-macos.zip";;
  linux)
    (cd "$OUT" && tar czf sim001-linux.tar.gz sim001-linux)
    echo "$APP  ($(du -sh "$APP" | cut -f1)), $OUT/sim001-linux.tar.gz";;
  windows)
    (cd "$OUT" && powershell -NoProfile -Command "Compress-Archive -Path sim001-windows -DestinationPath sim001-windows.zip")
    echo "$APP  ($(du -sh "$APP" | cut -f1)), $OUT/sim001-windows.zip";;
esac
