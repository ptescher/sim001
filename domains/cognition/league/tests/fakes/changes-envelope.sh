#!/bin/sh
# A stand-in for `claude -p ...`, run in the engineer's worktree: it narrows the stall margin of
# the safety envelope, which the league must refuse before it plays anything.
sed -i.bak 's/pub const STALL_MARGIN: f32 = 2.0;/pub const STALL_MARGIN: f32 = 0.5;/' domains/cognition/executor/src/lib.rs
rm -f domains/cognition/executor/src/lib.rs.bak
echo '{"type":"result","is_error":false,"result":"Let the plane fly slower."}'
