#!/bin/sh
# Xcode build phase: build the Rust station for the platform being built, and put the static
# library where the linker looks (build/rust/<platform>/).
set -e
export PATH="$HOME/.cargo/bin:$PATH"
case "$PLATFORM_NAME" in
  iphonesimulator) TARGETS="aarch64-apple-ios-sim" ;;
  iphoneos) TARGETS="aarch64-apple-ios" ;;
  *) echo "unsupported platform $PLATFORM_NAME" >&2; exit 1 ;;
esac
PROFILE_DIR=debug; PROFILE_FLAG=""
if [ "$CONFIGURATION" = "Release" ]; then PROFILE_DIR=release; PROFILE_FLAG="--release"; fi
cd "$SRCROOT/../../../.."
cargo build -p teleoperation_ios $PROFILE_FLAG --target $TARGETS
mkdir -p "$SRCROOT/build/rust/$PLATFORM_NAME"
cp "target/$TARGETS/$PROFILE_DIR/libteleoperation_ios.a" "$SRCROOT/build/rust/$PLATFORM_NAME/"
