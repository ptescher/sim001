#!/bin/bash
# Runs ArduPlane SITL with the JSON backend pointed at the T1-S sim on the host.
#   SIM_HOST  where t1s_sim --ardupilot is running (default: Docker Desktop's host alias)
#   HOME_LOC  lat,lon,alt,heading (default: CMAC model flying field, Canberra)
#   AIRCRAFT  the plane t1s_sim flies (default t1s-vtol, the only one with a flight controller):
#             /<AIRCRAFT>.parm, if there is one, is layered over the T1-S defaults
#   NOWAIT    1: do not wait for a ground station on the first serial port (the tests of the
#             sim alone have none). Always so for instances above 0
#   INSTANCE  0 (default) is the pilot's plane. 1 to 3 are the computer pilots' planes in the
#             sim's games: SITL -I N sends its physics to port 9002 + 10*N, and since no ground
#             station connects to one of those, it doesn't wait for one (--serial0 tcp:0)
set -e
SIM_HOST=${SIM_HOST:-host.docker.internal}
SIM_IP=$(getent ahostsv4 "$SIM_HOST" | awk 'NR==1{print $1}')
SIM_IP=${SIM_IP:-$SIM_HOST}
HOME_LOC=${HOME_LOC:--35.363261,149.165230,584,353}
AIRCRAFT=${AIRCRAFT:-t1s-vtol}
INSTANCE=${INSTANCE:-0}
DEFAULTS=/ardupilot/Tools/autotest/models/plane.parm,/t1s.parm
[ -f "/$AIRCRAFT.parm" ] && DEFAULTS="$DEFAULTS,/$AIRCRAFT.parm"
mkdir -p /sitl && cd /sitl
SERIAL=()
{ [ "$INSTANCE" != 0 ] || [ "${NOWAIT:-0}" = 1 ]; } && SERIAL=(--serial0 tcp:0)
echo "ArduPlane SITL $INSTANCE ($AIRCRAFT) -> JSON physics at $SIM_IP:$((9002 + 10 * INSTANCE)), MAVLink on tcp:$((5760 + 10 * INSTANCE))"
exec /ardupilot/build/sitl/bin/arduplane \
    --model "JSON:$SIM_IP" \
    --speedup 1 \
    --home "$HOME_LOC" \
    --defaults "$DEFAULTS" \
    "${SERIAL[@]}" \
    -I"$INSTANCE" "$@"
