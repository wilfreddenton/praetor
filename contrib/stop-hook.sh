#!/usr/bin/env bash
# Stop-hook fallback, for environments where Claude Code *channels* are not
# available: Bedrock / Vertex / Foundry, orgs that have not enabled channels,
# or anyone unwilling to run --dangerously-load-development-channels.
#
# Channels push events into a live session, so none of this is needed there.
# Without them, an agent can only be woken when a background task *exits*, which
# makes a listener one-shot: it must be re-armed after every event, and an agent
# that parks unarmed suffers a lost wakeup.
#
# This hook makes that impossible: on every Stop it checks whether a background
# task tagged for this agent is still running, and if not, tells Claude to
# re-arm before it may park.
#
# Register it in .claude/settings.json (exec form — no shell quoting):
#
#   { "hooks": { "Stop": [ { "hooks": [ {
#       "type": "command",
#       "command": "/path/to/stop-hook.sh",
#       "args": ["alice-listener", "curl -sN 'http://127.0.0.1:9440/recv?me=<id>' || sleep 30"]
#   } ] } ] } }
#
# Empirically verified (see experiments/):
#   * `background_tasks` arrives on stdin with each task's verbatim `command`
#   * completed tasks disappear from that array, so presence == liveness
#   * `additionalContext` continues the turn without a spurious "hook error",
#     under the same loop protections as `decision: "block"`
#   * Claude Code overrides the hook after 8 consecutive continuations, so this
#     cannot trap an agent
set -euo pipefail

TAG="${1:?usage: stop-hook.sh <tag> <command>}"
CMD="${2:?usage: stop-hook.sh <tag> <command>}"
MARKER="PRAETOR_TAG=${TAG}"

payload="$(cat)"

# Is a shell background task carrying our marker still running? The marker is an
# inert env-var assignment we prepend to the command, so it round-trips verbatim
# through the task registry. Matching a tag we planted beats matching incidental
# text in the command, which can be reformatted.
if jq -e --arg m "$MARKER" \
     '.background_tasks[]? | select(.type == "shell") | select((.command // "") | contains($m))' \
     <<<"$payload" >/dev/null 2>&1; then
  exit 0   # armed: allow the agent to park
fi

jq -n --arg cmd "$MARKER $CMD" '{
  hookSpecificOutput: {
    hookEventName: "Stop",
    additionalContext: ("Your event listener is not armed, so you would park deaf to incoming messages. Re-arm it exactly once as a background task (Bash with run_in_background):\n\n    " + $cmd + "\n\nWhen it returns, handle the message and re-arm it again.")
  }
}'
