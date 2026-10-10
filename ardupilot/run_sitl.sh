#!/bin/bash
# Build (first time) and run ArduPlane SITL in Docker, connected to t1s_sim --ardupilot.
# Connect a ground station (QGroundControl / Mission Planner / MAVProxy) to tcp:localhost:5760.
set -e
cd "$(dirname "$0")"
docker image inspect t1s-ardupilot-sitl >/dev/null 2>&1 || docker build -t t1s-ardupilot-sitl .
exec docker run --rm -it --name t1s-sitl -p 5760:5760 t1s-ardupilot-sitl "$@"
