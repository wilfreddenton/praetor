# interlink plugin

Bundles the whole interlink setup for Claude Code into one install: the MCP server
(via `npx interlink-mcp`), the two `PreToolUse` guard hooks, and the `read-only` /
`dev` capability agents. No hand-editing `settings.json`.

## Install

```
/plugin marketplace add wilfreddenton/interlink
/plugin install interlink@interlink
```

That registers, in every session:
- **MCP server** `interlink` — `send_message`, `fetch_request`, `add_peer`,
  `list_peers`, `remove_peer`, `message_status`, `conversation_history`,
  `list_pending`.
- **Hooks** — `pretooluse-guard.sh` (keeps `fetch_request` out of the main
  agent) and `peer-admin-guard.sh` (keeps `add_peer`/`remove_peer` out of
  subagents), wired via `${CLAUDE_PLUGIN_ROOT}`.
- **Agents** — `read-only` (inspect only) and `dev` (read + edit, no shell), the
  scoped capability presets you grant a peer to fence its tools.
- **Skill** — `interlink`, an on-demand playbook for operating the mesh
  (collaborating with a peer until a task completes, handling incoming messages,
  grants as the tool ceiling, onboarding peers via discover/pairing).

## One-time setup

The MCP config points at a standard config location; create your identity and
allowlist there:

```bash
mkdir -p ~/.config/interlink ~/.local/state/interlink
interlink-keygen --out ~/.config/interlink/id.key      # from `npx interlink-mcp`'s crate, or cargo install
printf '{}\n' > ~/.config/interlink/peers.json       # then add peers with add_peer
```

Set `INTERLINK_URL` in your environment if your bus isn't on `127.0.0.1:9440`.

To **receive** pushed messages, launch with the channel flag (research preview):

```bash
claude --dangerously-load-development-channels server:interlink
```

## Requirements

- **`interlink-mcp` on npm** (`npx -y interlink-mcp` resolves the pure-Rust binary).
  Or swap the `.mcp.json` `command` for a `cargo install`ed `interlink-mcp`.
- **A POSIX shell** for the guard hooks (macOS/Linux, or Git Bash on Windows).
  The MCP server and agent are cross-platform; the bash guards are the *nix
  enforcement layer.
