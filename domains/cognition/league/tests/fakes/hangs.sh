#!/bin/sh
# A stand-in for `claude -p ...` that never finishes: it waits on a child of its own, as Claude
# waits on a build, so that stopping only this shell would leave the child holding its output.
sleep 600 &
wait
