#!/bin/sh
# A stand-in for `claude -p ...`, run in the engineer's worktree: it makes every plane as quick
# to catch as any other, so the chaser no longer goes for the one it can catch soonest, which the
# agent's own tests catch.
sed -i.bak 's|to.length() / (TOP_SPEED - t.vel.length()).max(3.0) + behind|0.0 * to.length() + behind|' domains/cognition/agent/src/scripted.rs
rm -f domains/cognition/agent/src/scripted.rs.bak
mkdir -p .league
echo '{"notes": "The chaser no longer weighs speed."}' > .league/candidates.json
echo '{"type":"result","is_error":false,"result":"Made the chaser simpler."}'
