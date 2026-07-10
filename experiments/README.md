# experiments

Manual harnesses that establish how the Claude Code runtime *actually* behaves —
because the docs were incomplete or wrong on several points, and the whole design
rests on these facts. None of these are CI tests; they launch real Claude
sessions and cost tokens.

## Verified facts (mid-2026, Claude Code v2.1.20x)

- **A background task's completion wakes a *parked* agent — on exit code 0.**
  This is what the retired Stop-hook design relied on. (Now superseded by
  channels; see the git history and `contrib/stop-hook.sh`.)
- **`background_tasks` and `stop_hook_active`** arrive on the Stop hook's stdin;
  completed tasks vanish from the array, so presence == liveness.
- **A `PreToolUse` hook carries `agent_type` inside a subagent** (the custom
  agent's name), and is *absent* for the main agent — so a hook can tell which
  compartment a tool call comes from.
- **`PreToolUse` fires for MCP tools** (`mcp__server__tool`) and can **deny**
  them (`permissionDecision: "deny"`); the tool never runs and the model gets
  nothing.
- **Channels only arm with an interactive TTY.** Headless `claude -p` connects
  the MCP server but never engages the channel subsystem.
- **`rmcp` can be a channel**: declare `experimental: {"claude/channel": {}}`
  and push `CustomNotification::new("notifications/claude/channel", …)`.

## live_channel_test.py

Drives a live receiver session through a PTY, answers the two startup
confirmations, fires a signed peer message, and checks the full loop:
channel armed → event delivered → agent acted → signed reply reached the bus.

```bash
ESC_TEST_DIR=/path/to/workdir python3 live_channel_test.py
```

See the module docstring for the workdir layout.
