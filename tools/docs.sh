#!/bin/bash
# Runs every acceptance runner and writes the docs (docs/), with a hard time limit on each.
#   tools/docs.sh [--no-station]     --no-station skips the console's tests (they need Docker, and take a while)
# The runners, in the order the results are merged: the domains that run with no window (what the
# radios tell the station, the flight computer and the TX15's screen, the view of a real plane), the
# sim's own tests, the VTOL on its own and the sim with a flight controller (a real ArduPilot beside it) and the console's.
set -e
cd "$(dirname "$0")/.."
DOCS="$PWD/docs"
limit() { secs=$1; shift; perl -e 'alarm shift; exec @ARGV' "$secs" "$@"; }
echo "== domains with no window"
SIM001_DOCS="$DOCS" limit 900 cargo test --release -p communication_radio -p aviation_computer -p visualization_telemetry --test acceptance
echo "== the sim"
limit 900 cargo build --release
limit 480 ./target/release/sim001 --expect --docs "$DOCS" || true
echo "== the VTOL on its own (the sim starts ArduPilot; no --direct)"
SIM001_EXPECT_TIMEOUT=400 limit 1500 ./target/release/sim001 --aircraft t1s-vtol --expect --docs "$DOCS" || true
echo "== the sim with a flight controller (ArduPilot SITL)"
limit 400 ardupilot/expect_sim.sh --docs "$DOCS" || true
if [ "$1" != "--no-station" ]; then
  echo "== the console (the station: Docker, SITL, the sim and the console)"
  (cd domains/teleoperation/console && limit 2400 cargo run --release -- --station --expect --docs "$DOCS") || true
fi
echo "== the docs"
cargo run -q -p expectation_cli -- docs expectations "$DOCS"
