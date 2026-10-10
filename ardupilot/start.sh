#!/bin/bash
# Runs ArduPlane SITL with the JSON backend pointed at the T1-S sim on the host.
#   SIM_HOST  where t1s_sim --ardupilot is running (default: Docker Desktop's host alias)
#   HOME_LOC  lat,lon,alt,heading (default: CMAC model flying field, Canberra)
set -e
SIM_HOST=${SIM_HOST:-host.docker.internal}
SIM_IP=$(getent ahostsv4 "$SIM_HOST" | awk 'NR==1{print $1}')
SIM_IP=${SIM_IP:-$SIM_HOST}
HOME_LOC=${HOME_LOC:--35.363261,149.165230,584,353}
mkdir -p /sitl && cd /sitl
echo "ArduPlane SITL -> JSON physics at $SIM_IP:9002, MAVLink on tcp:5760"
exec /ardupilot/build/sitl/bin/arduplane \
    --model "JSON:$SIM_IP" \
    --speedup 1 \
    --home "$HOME_LOC" \
    --defaults /ardupilot/Tools/autotest/models/plane.parm,/t1s.parm \
    -I0 "$@"
