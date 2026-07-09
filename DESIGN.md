# Design notes

## The execution model that shapes everything

Claude Code runs a model in an agent loop. Two facts about that loop dictate this
entire design:

1. **The model acts on turns, then parks.** It does not run a background thread of
   its own. There is no "while listening" state inside the model, and no place to
   hang a callback.
2. **The harness re-invokes the model on discrete events.** The relevant one:
   *a background shell task (launched with `run_in_background`) exits.* When it
   does, its output is delivered and the model runs again.

So the only callback primitive available is **one-shot**: register by launching a
blocking background task; the "callback" is its exit. Everything below follows
from working *with* that instead of against it.

## Why not the obvious designs

**In-turn polling.** "Loop calling a `poll` tool until an event arrives." Never
yields the turn — spends tokens continuously, and the agent never returns to idle
where a human can talk to it. Rejected.

**One immortal listener.** "Launch a background task that loops forever reading
the bus." A background task only wakes the agent when it *exits*. A task that
never exits never wakes anyone; events accumulate unseen. Rejected.

**Timer polling (`/loop`, cron).** Genuinely viable: wake every N seconds and
drain the queue. Harness-owned, so liveness is guaranteed. The cost is latency
and a token tick per empty poll. `escapement` prefers event-driven delivery, but
the bus's `/recv` supports this style too.

## The chosen design: re-arm loop + Stop-hook guarantee

Receiving is a loop of short-lived listeners:

```
arm ──► curl /recv (blocks) ──► event arrives ──► curl exits ──► agent wakes
 ▲                                                                    │
 └──────────────────── agent handles it, re-arms ◄───────────────────┘
```

The weak point is "re-arms": it's a *model action*, and model actions are never
guaranteed. Parking without re-arming is a **lost wakeup**. So we move the
guarantee into the harness with a **`Stop` hook** (`escapement::hook`):

- On every stop, the hook asks the bus `GET /armed?me=<self>`.
- **Armed** (a `/recv` is in flight) → allow the park.
- **Not armed** → return `{"decision":"block","reason":"…re-arm…"}`, forcing the
  model to continue and re-arm before it can idle.

This is exactly the condition-variable discipline (`while !cond { wait() }`):
never park without having registered first.

Properties that make it safe:

- **Bounded.** It blocks only while disarmed. Once the agent re-arms, the next
  stop sees it armed and lets go — no infinite loop.
- **Fails open.** If the bus is unreachable, the hook allows the park. A dead bus
  can never trap the agent.
- **No lost events.** The bus queues per recipient, so the gap between one
  listener exiting and the next arming only *delays* delivery.

## The arming signal

"Armed" is defined as *the count of in-flight `/recv` calls for a recipient is
> 0*. The bus increments an `AtomicUsize` around the `await` in `Broker::recv`
and decrements after. This is more robust than the hook `pgrep`-ing for a curl
process: the bus is the single source of truth about whether a listener is
actually connected, not merely whether a process exists.

## Why sending is MCP but receiving is not

Sending is a discrete, model-initiated action → a natural MCP **tool**
(`send_message`). Receiving must *wake* the model from a parked state, and only a
background shell task can do that → it's a `curl`, outside MCP entirely. This is
why the `duet` MCP server sits only in the send path; the receive path talks to
the bus directly.

## Two protocols, one adapter

The `duet` binary translates between two client/server relationships:

- **Facing Claude:** an MCP server (JSON-RPC over stdio), via `rmcp`.
- **Facing the bus:** an HTTP client (HTTPS), via `reqwest`.

Claude only speaks MCP; the bus only speaks HTTP. Keeping them separate means the
bus is reusable by anything (a shell script, another language) and the MCP layer
carries no queueing logic.

## Crate shape: one crate, feature-gated

`escapement` is a single crate with `hook` / `bus` / `mcp` features rather than a
workspace of small crates. At this size a workspace is over-factoring, and the
modularity is preserved where it matters: optional dependencies mean the `hook`
feature pulls 18 transitive deps while the full set pulls 442. Binaries use
`required-features`, so a default `cargo install escapement` yields only
`escapement-hook`.

Features are strictly **additive** (never mutually exclusive), because Cargo
unifies features across a dependency graph. CI runs `cargo hack
--feature-powerset` so a `#[cfg(feature = "…")]` typo can't pass locally and
break for a user on a different feature set.

## Transport choices

- **stdio for MCP.** Claude Code launches the server as a subprocess; stdio is the
  standard local transport. No HTTP server involved on the MCP side.
- **HTTPS for the bus,** terminated with `tokio-rustls` and served by
  `hyper-util`'s connection builder driving an axum `Router` — the canonical way
  to serve axum over rustls without the `axum-server` wrapper. `ring` is the
  crypto provider throughout, so there's no `aws-lc-rs`/`cmake` build dependency.

## Prior art

The mechanism is not novel in computing — only in this setting:

- **systemd socket activation / inetd.** A supervisor holds the armed socket. An
  event arrives; the supervisor starts the otherwise-not-running service; the
  service handles it and exits; the supervisor re-arms. That is this architecture
  line for line, with the Stop hook as the supervisor.
- **Rust's `Waker` / `park`-`unpark`.** A `Future` that isn't ready registers a
  waker and the task parks; the reactor calls `wake()` when the event lands.
  Parking without registering is a lost wakeup.
- **Condition variables.** `while !cond { wait(&mutex) }` exists precisely to make
  "check, then park" atomic. The Stop hook is that atomicity, imposed externally.

Within Claude Code, `/goal` is the closest cousin: it is also a Stop-hook-shaped
mechanism that blocks the stop and forces another turn — but its predicate is *"is
my completion condition met?"* rather than *"is my listener armed?"*, and it never
crosses agent boundaries.
