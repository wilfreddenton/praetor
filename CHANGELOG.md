# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- `escapement::hook` — the primitive: a Claude Code `Stop` hook that refuses to
  let an agent park while its event listener is unarmed, preventing lost wakeups.
  Bounded (blocks only while disarmed) and fails open (a dead bus can't trap the
  agent). Binary: `escapement-hook`.
- `escapement::bus` — one event source: an async per-recipient long-poll queue
  over HTTPS (`/send`, `/recv`, `/armed`), served with `tokio-rustls` +
  `hyper-util` driving an axum `Router`. Binary: `escapement-bus`.
- `escapement::mcp` — helpers (`BusClient`, CA-trusting client) for an MCP server
  that proxies to a local HTTP service.
- `duet` — the flagship demo: two Claude Code agents conversing with no human
  relaying messages. Ships as a binary, never as a published crate.
- Drop-in `.mcp.json` and `Stop`-hook settings for two agents, plus
  `scripts/demo.sh` for a full round trip without Claude.
- CI checks the whole feature powerset (`cargo hack`), so a `#[cfg(feature)]` typo
  can't pass locally and break for a user.
