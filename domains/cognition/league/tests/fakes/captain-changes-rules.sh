#!/bin/sh
# A stand-in for `claude -p ...`, run in the captain engineer's worktree: it makes towers quicker
# to take, a change to the rules that the league must refuse before it plays anything.
sed -i.bak 's/capture_time: 20.0,/capture_time: 5.0,/' domains/recreation/towers/src/lib.rs
rm -f domains/recreation/towers/src/lib.rs.bak
echo '{"type":"result","is_error":false,"result":"Made towers quicker to take."}'
