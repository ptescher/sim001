#!/bin/bash
# Plays N recorded Towers games one after another, headless, and lists their crashes:
#   tools/flightlog/games.sh N OUT [towers_server arguments...]
# Each game is OUT/g<k> (see `towers_server --record`), its server output OUT/g<k>.log, and one
# line in OUT/summary.txt. A game with no crash keeps its tracks but not its ArduPilot logs (about
# 300 MB a game). Needs the AGENT build (SIM001_BIN, else .ardupilot-agent) and a release build of
# the server (cargo build --release -p recreation_towers_server). One game at a time: a game is ten
# ArduPlane SITLs, and a busy machine makes drones crash on its own.
set -u
N=$1; OUT=$2; shift 2
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "$OUT"
for k in $(seq 1 "$N"); do
  G=$OUT/g$k
  rm -rf "$G"
  "$ROOT/target/release/towers_server" --speedup 10 --timeout 350 --record "$G" "$@" > "$G.log" 2>&1
  crashes=$(($(wc -l < "$G/crashes.tsv") - 1))
  echo "game $k crashes $crashes $(grep -o 'RESULT outcome=[A-Za-z]*' "$G.log")" | tee -a "$OUT/summary.txt"
  [ "$crashes" -eq 0 ] && rm -f "$G"/*.BIN
done
