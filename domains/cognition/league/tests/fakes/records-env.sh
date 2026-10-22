#!/bin/sh
# A stand-in for `claude -p ...` that records the names of the Claude variables it was given.
mkdir -p .league
env | grep -E '^(CLAUDE|ANTHROPIC_BASE_URL)' | cut -d= -f1 > .league/env.txt
echo '{"type":"result","is_error":false,"result":"looked at the environment"}'
