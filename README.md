# escapement

[![CI](https://github.com/wilfreddenton/escapement/actions/workflows/ci.yml/badge.svg)](https://github.com/wilfreddenton/escapement/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)

**A durable, self-re-arming event listener for Claude Code agents.**

An agent that goes idle without an armed listener suffers a **lost wakeup**.
`escapement` makes that impossible.

## The problem

A Claude Code agent is not a persistent process. It exists only during a turn,
then parks. So there is nowhere to hang a callback, and the harness offers
exactly one wake primitive:

> **An agent is re-invoked when a background shell task *exits*.**

That gives you a **one-shot** event listener — the equivalent of
`addEventListener(…, { once: true })`. And a one-shot listener must be
re-registered after every event. Which means the two obvious designs both fail:

- **Poll inside the turn** → never yields. Burns tokens, never returns to idle.
- **An eternal `while true` background listener** → never *exits*, so it never
  wakes anyone. Events pile up unread.

And the subtle one: if the agent parks *without* re-registering, the next event
is silently missed. That's a **lost wakeup** — the classic concurrency bug that
condition variables exist to prevent.

## The mechanism

```
        ┌──── agent parks (idle) ────┐
        │                            │
   Stop hook: armed?            listener blocks on /recv
   ├─ yes → allow park               │
   └─ no  → BLOCK, re-arm       event arrives → listener EXITS
        ▲                            │
        └──── agent wakes, handles ──┘
```

1. The agent arms a listener: a background task that blocks until an event
   arrives, then **exits** — and *that exit is the wake*.
2. It handles the event and re-arms.
3. A **`Stop` hook** refuses to let the agent park while unarmed. Liveness lives
   in the harness, never in the model's memory.

Like a watch escapement: it locks the train, releases exactly one impulse, and
re-locks.

The hook is **bounded** (it blocks only while disarmed, so it can't loop forever)
and **fails open** (an unreachable bus allows the park, so a dead bus can never
trap the agent). Events are queued per-recipient, so a missed re-arm *delays*
delivery — it never loses it.

## Install

One crate, feature-gated. You only compile what you use.

```bash
cargo install escapement                      # the hook (default)
cargo install escapement --features full      # hook + bus + the demo
```

| Feature | Provides | Binary |
|---|---|---|
| `hook` *(default)* | the Stop hook — **the primitive** | `escapement-hook` |
| `bus` | one event source: a per-recipient long-poll queue over HTTPS | `escapement-bus` |
| `mcp` | helpers for an MCP server that proxies to a local HTTP service | `duet` |
| `full` | all of the above | — |

Features are additive and dependencies are gated: `escapement` with only `hook`
pulls **18** transitive deps; with everything, 442. As a library:

```rust
use escapement::hook::{check_armed, block_decision, default_listen_cmd};
```

## The demo: two agents in conversation

`duet` is the flagship example — two Claude Code instances talking with no human
relaying messages. Agent *alice* sends via an MCP tool; agent *bob* is woken by
his armed listener, replies, and re-arms.

```bash
cargo build --release --features full
./target/release/escapement-bus       # the event source (generates certs/ on first run)
./scripts/demo.sh                     # full round trip, no Claude required
```

To wire up real instances, copy `config/alice.mcp.json` → `<alice>/.mcp.json` and
merge `config/settings.alice.json` into `<alice>/.claude/settings.json` (same for
`bob`), then restart both. Tell each once: *"arm your listener."* The Stop hook
keeps it armed forever after.

The bus is just *one* event source. The same primitive wakes an agent on a
webhook, a CI result, a queue message, or a file change.

## Related work

`escapement` is often mistaken for things it isn't:

| | What it does | Cross-agent? |
|---|---|---|
| **`/goal`** | keeps *one* session looping until a condition holds | no |
| **`/loop`** | re-runs a prompt on a time interval | no |
| **Subagents** | spawns children *inside* one session | no |
| **Agent Teams** | coordinates agents within one conversation | no |
| **`escapement`** | makes an agent **reactive to external events** | yes |

The nearest prior art isn't in the agent world at all — it's **systemd socket
activation** (a supervisor holds the armed socket, wakes an inactive service on
an event, and re-arms), and **Rust's own `Waker`** (a parked task registers a
waker; parking without one is a lost wakeup). `escapement` is that discipline,
applied to an agent.

## Design

The full walkthrough — execution model, rejected designs, the arming signal, and
failure modes — is in [`DESIGN.md`](DESIGN.md). Planned signed identity is in
[`DIRECTORY.md`](DIRECTORY.md).

## Limits

- **Not parallel.** An agent does one thing at a time; an incoming event waits for
  the current turn to finish.
- **Heartbeat wakes.** With no traffic the long-poll times out (~5 min), wakes the
  agent, and re-arms — a periodic no-op.
- **Context growth.** A long-lived agent accumulates context per event.
- **Local + self-signed.** The bus binds `127.0.0.1` with an `rcgen` cert. Not
  built to face a network.

## License

MIT — see [LICENSE](LICENSE).
