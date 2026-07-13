# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- `interlink-bus` — a loopback HTTP broker with one bounded FIFO per recipient
  key; buffers for offline agents, holds no keys, verifies nothing.
- `interlink-mcp` — the per-agent Claude Code **channel** server. Long-polls
  the bus and, for each message, runs the inbound gate (verify signature →
  allowlist → addressed-to-me → fresh → dedupe) before pushing
  `notifications/claude/channel`. Tools: `send_message`, `fetch_request`,
  `message_status`, `conversation_history`, `list_pending`, and live peer
  management (`add_peer`, `list_peers`, `remove_peer`).
- `interlink-keygen` — generate an Ed25519 identity; the public key is the id.
- **`identity`** — Ed25519, public-key-as-identity, domain-separated signing,
  `verify_strict`, freshness + replay protection.
- **`policy`** — `peers.json`: per-peer grant of `"*"` (inline) or a capability
  name (scoped to an agent's `tools:` frontmatter). Deny-by-default.
- **Scoped enforcement** — untrusted bodies are quarantined and reachable only
  via `fetch_request`, gated to subagents by
  [`contrib/pretooluse-guard.sh`](contrib/pretooluse-guard.sh); tool limits are
  the capability agent's frontmatter.
- **Live peer management** — `add_peer` / `list_peers` / `remove_peer` edit the
  allowlist from chat, persisted to `peers.json` and applied immediately (the
  inbound gate re-reads it per message). Kept out of scoped subagents by
  [`contrib/peer-admin-guard.sh`](contrib/peer-admin-guard.sh) so an untrusted
  peer can't escalate itself onto the allowlist; the main agent still gets
  Claude Code's normal permission prompt.
- **Discovery & pairing** (v0.2.0) — a bus **roster** (`/announce` + `/roster`,
  TTL presence) plus a `discover` tool; and a human-gated **pairing handshake**
  (`request_pair` / `list_pair_requests` / `accept_pair` / `reject_pair`) so nodes
  boot with an empty `peers.json` and connect at runtime. A non-peer may only
  *knock* (identity + self-claimed name, no free text); everything else from a
  non-peer is still dropped. Grants are per-side; keys are pinned TOFU; the accept
  tools are operator-only (subagent-guarded). Adds a signed `kind` on messages,
  which bumps the signing domain `interlink-v1 → v2` (a breaking change; all nodes
  must be ≥0.2.0). Design: [`docs/DISCOVERY.md`](docs/DISCOVERY.md).
- **Named inboxes (labels)** — a session launches with `INTERLINK_LABEL=<name>` and
  receives only messages addressed to it via `send_message`'s `channel`. Lets
  several sessions share one identity, each with its own addressable stream.
  Routing is `key#label`; the signature still binds the bare key, so the trust
  gate is untouched and the bus needs no changes. Per-endpoint sub-addressing on
  one key is novel among agent-chat MCP servers.
- Live integration harnesses in [`experiments/`](experiments) that drive a real
  Claude session through a PTY, plus the runtime facts they established.
- `contrib/stop-hook.sh` — the pre-channels fallback, for environments where
  channels can't run.

- `interlink-mcp` reconnects to the bus automatically after a sleep/reboot/crash
  (retry-forever long-poll, no socket reuse across wake) and never crashes when
  the bus is absent. A systemd user service (`contrib/interlink-bus.service`)
  auto-starts the bus on boot.
- **`persist`** — a durable, keep-until-acked FIFO queue over
  [redb](https://crates.io/crates/redb) (pure Rust, ACID; no C). Shared by both
  sides: the **bus** (`--db`/`INTERLINK_DB`) holds a message for an offline
  recipient until it acks, and each **agent** (`INTERLINK_AGENT_DB`) holds an
  unsent message in a durable outbox until the bus accepts it — so a restart of
  either loses nothing. Delivery is at-least-once, made safe by the existing
  `msg_id` dedupe. The agent's store also keeps a local conversation log,
  queried by the `message_status` / `conversation_history` / `list_pending`
  tools; scoped/untrusted peers' bodies are recorded as metadata only and never
  written to disk.

### Distribution
- **Claude Code plugin** ([`plugin/`](plugin)) — bundles the MCP server, both
  guard hooks, the `read-only` / `dev` capability agents, and the `interlink` skill
  (continuous collaboration as the default mode; grants as the tool ceiling);
  installable via a marketplace (`/plugin marketplace add wilfreddenton/interlink`).
- **npm wrapper** ([`npm/`](npm)) — `npx interlink-mcp` fetches the platform's
  prebuilt static binary (the esbuild/Biome model), so the pure-Rust core gets
  the `npx` ergonomics the MCP ecosystem expects.
- **Release workflow** — a tag builds and publishes static binaries for Linux
  (x86_64/aarch64 musl), macOS (aarch64), and Windows (x86_64); the no-C
  invariant keeps every target a native-runner build, no cross toolchain.

### Notable choices
- **No TLS**: loopback bus + signed messages; removes the only C dependency, so
  binaries are pure-Rust and statically linkable.
- CI fails the build if a C dependency (`ring`/`openssl-sys`/`cc`/`cmake`)
  reappears, and checks the whole feature powerset.
