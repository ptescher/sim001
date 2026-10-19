#!/bin/bash
# Builds ArduPlane SITL natively (no Docker), into .ardupilot/ in the repo. The sim starts it
# itself when you fly a plane with a flight controller: `cargo run --release` (the T1-S VTOL).
# One time, a few minutes: it clones ArduPilot (shallow, with its submodules), applies our patch
# series (ardupilot/patches/, the AGENT mode) and compiles the simulator. Needs Python 3 and a
# C++ compiler (Xcode's command line tools on a Mac).
#   ardupilot/build_native.sh [AP_REF]      AP_REF is a branch or tag (default: the pinned release)
#   AP_DIR=.ardupilot-x ardupilot/build_native.sh     build somewhere else (default .ardupilot)
#   AGENT_EXEC=1 ardupilot/build_native.sh            link the Rust plan executor (domains/cognition/executor)
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
# Our patch series, in order. A reused tree remembers what it has in .git/sim001-patches (name and
# hash); a patch already in it is skipped. A tree from before that file: a patch that reverses
# cleanly is taken as in. A patch that changed since it went in: start from a clean tree
# (git checkout . && git clean -fd ArduPlane, in $DIR), and it all goes in again.
STAMP="$(git rev-parse --git-dir)/sim001-patches"
touch "$STAMP"
for p in "$ROOT"/ardupilot/patches/*.patch; do
  [ -e "$p" ] || continue
  entry="$(basename "$p") $(git hash-object "$p")"
  if grep -qxF "$entry" "$STAMP"; then
    echo "patch already applied: $(basename "$p")"
  elif git apply --reverse --check "$p" 2>/dev/null; then
    echo "patch already applied: $(basename "$p")"
    echo "$entry" >> "$STAMP"
  elif git apply "$p"; then
    echo "patch applied: $(basename "$p")"
    echo "$entry" >> "$STAMP"
  else
    echo "patch $(basename "$p") does not apply to $DIR; if an older version of it is in, start from a clean tree" >&2
    exit 1
  fi
done
# The executor's C ABI has one source of truth, domains/cognition/executor/include; the AGENT mode
# includes this copy of it (with or without AGENT_EXEC: without, mode_agent.cpp stands in a
# wings-level executor behind the same ABI). It is not in the patch series, so it can't go stale.
cp "$ROOT/domains/cognition/executor/include/agent_executor.h" ArduPlane/agent_executor.h
# Apple's linker refuses the pointers inside this packed struct (they sit unaligned); SITL doesn't
# need it packed.
[ "$(uname)" = Darwin ] && sed -i '' 's/^class PACKED AP_FWVersion/class AP_FWVersion/' libraries/AP_Common/AP_FWVersion.h
if [ "${AGENT_EXEC:-0}" = 1 ]; then
  (cd "$ROOT" && cargo build --profile embedded -p cognition_executor --features ffi-static)
  export CXXFLAGS="${CXXFLAGS:-} -DAP_AGENT_EXEC=1"
  export LDFLAGS="${LDFLAGS:-} $ROOT/target/embedded/libcognition_executor.a"
  # waf doesn't track a library passed in LDFLAGS: a newer executor forces the relink
  if [ build/sitl/bin/arduplane -ot "$ROOT/target/embedded/libcognition_executor.a" ]; then
    rm -f build/sitl/bin/arduplane
  fi
fi
./waf configure --board sitl
./waf plane
echo "built: $(pwd)/build/sitl/bin/arduplane"
