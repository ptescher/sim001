#!/bin/bash
# Builds ArduPlane SITL natively (no Docker), into .ardupilot/ in the repo. The sim starts it
# itself when you fly a plane with a flight controller: `cargo run --release` (the T1-S VTOL).
# One time, a few minutes: it clones ArduPilot (shallow, with its submodules), applies our patch
# series (ardupilot/patches/, the AGENT mode) and compiles the simulator. Needs Python 3 and a
# C++ compiler (Xcode's command line tools on a Mac).
#   ardupilot/build_native.sh [AP_REF]      AP_REF is a branch or tag (default: the pinned release)
#   AP_DIR=.ardupilot-x ardupilot/build_native.sh     build somewhere else (default .ardupilot)
#   AGENT_EXEC=1 ardupilot/build_native.sh            link the Rust plan executor (domains/autonomy/executor)
#                                                      into the AGENT mode; without it the mode is a stub
set -e
cd "$(dirname "$0")/.."
ROOT=$PWD
REF=${1:-Plane-4.7.2}
DIR=${AP_DIR:-.ardupilot}
if [ ! -d "$DIR/.git" ]; then
  git clone --depth 1 --branch "$REF" --recurse-submodules --shallow-submodules https://github.com/ArduPilot/ardupilot.git "$DIR"
fi
[ -d .ardupilot-venv ] || python3 -m venv .ardupilot-venv
. .ardupilot-venv/bin/activate
pip install --quiet empy==3.3.4 pexpect future setuptools
cd "$DIR"
# Our patch series, in order; one that is already in (the tree is reused) is skipped.
for p in "$ROOT"/ardupilot/patches/*.patch; do
  [ -e "$p" ] || continue
  if git apply --reverse --check "$p" 2>/dev/null; then
    echo "patch already applied: $(basename "$p")"
  else
    git apply "$p" && echo "patch applied: $(basename "$p")"
  fi
done
# Apple's linker refuses the pointers inside this packed struct (they sit unaligned); SITL doesn't
# need it packed.
[ "$(uname)" = Darwin ] && sed -i '' 's/^class PACKED AP_FWVersion/class AP_FWVersion/' libraries/AP_Common/AP_FWVersion.h
if [ "${AGENT_EXEC:-0}" = 1 ]; then
  (cd "$ROOT" && cargo build --profile embedded -p autonomy_executor --features ffi-static)
  export CXXFLAGS="${CXXFLAGS:-} -DAP_AGENT_EXEC=1"
  export LDFLAGS="${LDFLAGS:-} $ROOT/target/embedded/libautonomy_executor.a"
fi
./waf configure --board sitl
./waf plane
echo "built: $(pwd)/build/sitl/bin/arduplane"
