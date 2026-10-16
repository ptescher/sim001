#!/bin/bash
# Runs the sim's own acceptance tests for a plane with a flight controller (the T1-S VTOL): the
# section "Flying with a flight controller". The sim flies from its sticks (--direct) while a real
# ArduPlane SITL steps the physics beside it, so the tests (and their screenshots) are of the sim
# as it runs with ArduPilot. The plain T1-S's tests are `cargo run --release -- --expect`.
#   ardupilot/expect_sim.sh [--docs DIR] [--only TEXT]      (run from anywhere)
set -e
cd "$(dirname "$0")/.."
docker image inspect t1s-ardupilot-sitl >/dev/null 2>&1 || docker build -q -t t1s-ardupilot-sitl ardupilot
docker rm -f t1s-sitl >/dev/null 2>&1 || true
docker run -d --rm --name t1s-sitl -p 5760:5760 -e AIRCRAFT=t1s-vtol -e NOWAIT=1 t1s-ardupilot-sitl >/dev/null
trap 'docker rm -f t1s-sitl >/dev/null 2>&1' EXIT
args=()
while [ $# -gt 0 ]; do
  case "$1" in
    --docs) args+=(--docs "$2"); shift 2 ;;
    --only) export T1S_EXPECT_ONLY="$2"; shift 2 ;;
    *) args+=("$1"); shift ;;
  esac
done
cargo run --release -- --aircraft t1s-vtol --ardupilot --direct --expect "${args[@]}"
