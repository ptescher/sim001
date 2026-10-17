#!/bin/bash
# Builds ArduPlane SITL natively (no Docker), into .ardupilot/ in the repo. The sim starts it
# itself when you fly a plane with a flight controller: `cargo run --release` (the T1-S VTOL).
# One time, a few minutes: it clones ArduPilot (shallow, with its submodules) and compiles the
# simulator. Needs Python 3 and a C++ compiler (Xcode's command line tools on a Mac).
#   ardupilot/build_native.sh [AP_REF]      AP_REF is a branch or tag (default master)
set -e
cd "$(dirname "$0")/.."
REF=${1:-master}
if [ ! -d .ardupilot/.git ]; then
  git clone --depth 1 --branch "$REF" --recurse-submodules --shallow-submodules https://github.com/ArduPilot/ardupilot.git .ardupilot
fi
[ -d .ardupilot-venv ] || python3 -m venv .ardupilot-venv
. .ardupilot-venv/bin/activate
pip install --quiet empy==3.3.4 pexpect future setuptools
cd .ardupilot
# Apple's linker refuses the pointers inside this packed struct (they sit unaligned); SITL doesn't
# need it packed.
[ "$(uname)" = Darwin ] && sed -i '' 's/^class PACKED AP_FWVersion/class AP_FWVersion/' libraries/AP_Common/AP_FWVersion.h
./waf configure --board sitl
./waf plane
echo "built: $(pwd)/build/sitl/bin/arduplane"
