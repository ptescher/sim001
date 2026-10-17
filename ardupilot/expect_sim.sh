#!/bin/bash
# Runs the sim's own acceptance tests for a plane with a flight controller (the T1-S VTOL): the
# section "Flying with a flight controller". The sim flies from its sticks (--direct) while a real
# ArduPlane SITL steps the physics beside it, so the tests (and their screenshots) are of the sim
# as it runs with ArduPilot. The plain T1-S's tests are `cargo run --release -- --expect`.
#   ardupilot/expect_sim.sh [--docs DIR] [--only TEXT]      (run from anywhere)
set -e
cd "$(dirname "$0")/.."
# A native build (ardupilot/build_native.sh) is started by the sim itself; otherwise Docker runs it.
ap=()
if [ ! -x .ardupilot/build/sitl/bin/arduplane ]; then
  docker image inspect sim001-ardupilot-sitl >/dev/null 2>&1 || docker build -q -t sim001-ardupilot-sitl ardupilot
  docker rm -f sim001-sitl >/dev/null 2>&1 || true
  docker run -d --rm --name sim001-sitl -p 5760:5760 -e AIRCRAFT=t1s-vtol -e NOWAIT=1 sim001-ardupilot-sitl >/dev/null
  trap 'docker rm -f sim001-sitl >/dev/null 2>&1' EXIT
  ap=(--ardupilot)
fi
args=()
while [ $# -gt 0 ]; do
  case "$1" in
    --docs) args+=(--docs "$2"); shift 2 ;;
    --only) export SIM001_EXPECT_ONLY="$2"; shift 2 ;;
    *) args+=("$1"); shift ;;
  esac
done
cargo run --release -- --aircraft t1s-vtol "${ap[@]}" --direct --expect "${args[@]}"
