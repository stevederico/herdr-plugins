#!/usr/bin/env bash
# Open (or replace) the agents popup picker.
set -euo pipefail
herdr="${HERDR_BIN_PATH:-herdr}"
delta="${1:-}"
export HERDR_AGENT_PICKER_DELTA="$delta"
exec "$herdr" plugin pane open \
  --plugin herdr-agent-picker \
  --entrypoint picker \
  --placement popup \
  --focus \
  --env "HERDR_AGENT_PICKER_DELTA=$delta"
