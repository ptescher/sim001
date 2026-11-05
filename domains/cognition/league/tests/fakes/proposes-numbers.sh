#!/bin/sh
# A stand-in for `claude -p ...` as a tuner: it tries a candidate, proposes it, and says what the
# run cost, as `claude --output-format json` does.
mkdir -p .league
echo '{"name": "try1", "tactics": {"lead": 0.9}}' > .league/try1.json
echo '[{"tactics": {"lead": 0.9}, "notes": "Lead the target a little less."}]' > .league/candidates.json
echo '{"type":"result","is_error":false,"result":"Proposed less lead.","total_cost_usd":0.42,"num_turns":7,"duration_ms":95000}'
