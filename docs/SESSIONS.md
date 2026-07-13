# Sessions, addressing & capability escalation (design)

> Status: **proposed**. Nothing here is built yet. Supersedes the current
> `PRAETOR_LABEL` mechanism (manual, collision-prone) and the
> pairing-time-frozen grant.

Today one identity (`id.key`) equals one machine, and everything sent to that key
arrives on one inbox. Running several Claude sessions on a machine breaks: they
all long-poll the bare key, so a peer's message is delivered to *some
unpredictable one* of them, occasionally *two* (delivery is peek-until-ack over a
shared FIFO, and dedupe is per-process). `PRAETOR_LABEL` was a first fix, but it
puts *uniqueness* on the human — two sessions with the same label collide again,
silently. And the capability grant is frozen into `peers.json` at pairing, so it's
machine-wide: you can't expose different capabilities from different sessions.

This design makes the **session** the addressable, capability-bearing unit, while
keeping **identity/trust** machine-wide — and makes least privilege the default.

## The invariant this must not break

The capability ceiling must be a **hard** wall, not a request the model may talk
itself past. Concretely:

> **Untrusted peer text never reaches an agent that holds tools beyond the current
> ceiling.** A peer's messages are handled by a delegate capped at that ceiling;
> the operator's own all-tools session never ingests the raw peer body. "Message
> only" therefore means a delegate with *no* action tools, not the main agent
> choosing to behave.

This is the same principle as today's quarantine (`fetch_request` reachable only
from a capped subagent), generalized to the whole conversation.

## Two planes

Everything splits into a **control plane** (identity) and a **data plane**
(session). They have different addresses, lifetimes, and trust semantics.

| | Control plane | Data plane |
|---|---|---|
| Address | bare `key` | `key#<session-token>` |
| Carries | knocks, accepts, escalation requests/grants | the task conversation |
| Scope | the machine (identity) | one session |
| Lifetime | permanent | that session's life |
| Delivery | loose is fine (low-volume, human-gated, idempotent) | must be deterministic + sticky |

Mental model: **the bare key is the machine's front door; `key#token` is a
specific session's phone line.** You knock at the front door once; you talk on the
session lines forever after.

## 1. Auto-unique session addressing

At startup each MCP session mints a **unique session token** (random) with no human
input, and polls `key#<token>` — never the bare key for data. Collisions are
impossible because no human chooses the token. The token also derives a
per-session store path, so concurrent sessions never fight over one redb file.

- Kills the `PRAETOR_LABEL` footgun entirely — there is no label to fat-finger.
- `PRAETOR_LABEL` (if kept at all) becomes only a **display alias** announced in
  presence; it is never the routing key, so a duplicate alias can't cause a
  shared inbox.

## 2. Reply-stickiness

Cold-addressing a specific remote session by name is the rare case; **pinning a
conversation once it starts** is the common one. So every message carries a
`reply_to` routing hint = the sender's `key#<token>`. The recipient's reply is
addressed to that exact session line, and the conversation stays point-to-point —
like ephemeral ports in TCP, where nobody ever names the port.

`reply_to` is an **unsigned routing hint**, exactly like the label suffix today:
the signature still binds the bare recipient key, so the trust gate is untouched
and a relay can at worst misroute among a recipient's own sessions.

## 3. Control inbox = the front door for cold contact

A knock is the ultimate cold message: the sender knows only the *identity*, no
session. Reply-stickiness can't help it (no prior conversation to stick to). So the
**bare-key inbox is reserved as the control inbox**, and every session *also*
polls it. Knocks, accepts, and escalation requests/grants land there; the
initiator's `reply_to` carries the answer back to the exact session that started
it.

Loose delivery here is acceptable — control traffic is low-volume, human-gated,
and idempotent to surface — so no new bus semantics are needed. (`notify_one`
usually surfaces a knock in just one session, which is the nicer behavior anyway.)

## 4. Admission vs. ceiling

The current grant does two jobs at once. Split them:

- **Admission** — "may this identity reach me at all?" Identity-level, machine-wide,
  permanent. Set once at pairing, stored in the shared `peers.json`. Pair once,
  every session (present and future) inherits it. Admission grants *no* ability to
  act — it only opens the door.
- **Ceiling** — "what may they *do* when they reach a given session?" Per-session,
  ephemeral. Enforced entirely on the receive side (which delegate/toolset handles
  the message), so it needs **no protocol** in the basic direction: the peer just
  sends; my session handles within *its* ceiling.

## 5. Least-privilege floor + escalation

Every session boots at the **floor: message-only.** An admitted peer can converse,
but can cause *nothing* on the machine — handled by a delegate with only
`send_message` + read-the-thread, no action tools.

To do more, a session **requests** a capability; the peer's operator approves or
denies (human-gated, surfaced in **one** session, not broadcast). A grant is
scoped to the **specific pair of sessions** in the conversation — the local
`key#token` *and* the remote `key#token` — and **evaporates when either ends**.
If either side reconnects as a new session, that pair is new and starts back at
the floor; only *admission* is inherited across sessions, never capability. No
standing promotion (a "remember this" convenience is deferred — explicit, off by
default).

A granted ceiling is **advertised to the peer** (told-upfront): its delegate is
told its limits in advance, so it escalates *proactively* rather than blindly
attempting a tool it doesn't have. This is a small control-plane handshake.

This is the *same handshake shape as pairing, one layer down*:

| Handshake | Admits/grants | Scope | Lifetime |
|---|---|---|---|
| **knock → accept** | an identity | machine | permanent |
| **request → grant** | a capability | session/conversation | ephemeral |

Both are signed, surface to the operator, and are refusable.

## 6. The backgrounded capped delegate

Where the peer conversation actually *runs*:

```
   You  ⇄  Main session           ← you; full tools; supervises + steers
              │
              ▼
        Peer-conversation delegate  ← backgrounded, capped at the ceiling
              ⇅
            Peer
```

- The **delegate** owns the whole peer conversation, both directions, capped at the
  current ceiling. It runs in the background.
- The **main session** is the operator's console: you steer through it ("tell the
  desktop to focus on X"), and it surfaces what matters back to you.
- **What surfaces:** escalation requests to approve, `✅ COMPLETE`, or genuine
  uncertainty. Routine back-and-forth stays backgrounded (peekable, not spammy).

Continuity — the thing that keeps this a *conversation*, not "run this on the
other machine":

- Within a ceiling level, the delegate **stays alive** and is fed each new peer
  message with its context intact (continue-a-subagent, not one-shot). One
  continuous mind holds the thread.
- On **escalation**, the delegate is re-spawned at the new toolset and re-briefed
  from the durable conversation log. Continuity is preserved by replay *only at
  escalation boundaries* — rare, human-gated events — while live context carries
  between them.

The spectrum is one mechanism:

- **message-only (floor)** → delegate with `send_message` + read-thread. Pure talk.
- **a capability** → same delegate re-spawned with that toolset. Talk + act within
  the cap.
- **`*`** → top of the ladder: a delegate with **all** tools. Full trust still
  runs *in the delegate* — the main session never ingests raw peer text, no
  exceptions. `*` widens the cap to everything; it does not move the conversation
  into your main context.

## What this delivers

The original ask — *"two sessions communicating back and forth without me in the
middle until a natural stopping point"* — made **safe**. The delegate is the thing
that talks to the peer without you in the middle; the ceiling is what makes
"without you in the middle" safe (it physically can't exceed message-only until you
approve more); the main chat is where you're pulled back in only at the natural
stopping points — escalations and completion. The only foreground activity is trust
decisions: pairing (once) and escalations (rare).

## Threat model / bounding

- **Ceiling is hard**, not advisory: raw peer text never enters an agent with tools
  beyond the ceiling. Injection can at worst act *within* the current cap.
- **Escalation is human-gated** and ephemeral; a compromised/confused peer can ask,
  never grant itself.
- **`reply_to` and session tokens are unsigned routing hints**: worst case a
  malicious relay misroutes among a recipient's own sessions — never to another
  identity, never past the trust gate.
- **Admission unchanged**: deny-by-default; a non-peer can still only knock.
- **Control inbox** carries only control traffic; loose delivery there is safe by
  construction (idempotent, human-gated).

## Build order

1. Auto-unique session token + per-session store path; sessions poll
   `key#<token>` + the control inbox. (Removes the label footgun; no behavior
   change for a single session.)
2. `reply_to` routing hint + reply-stickiness. Conversations pin correctly.
3. Split policy: admission (shared `peers.json`) vs. per-session ceiling; floor =
   message-only.
4. The backgrounded capped delegate + continue/replay lifecycle.
5. Escalation handshake (`request → grant`) on the control plane, human-gated,
   ephemeral, per session-pair; ceiling advertised to the peer (told-upfront).
6. (Deferred) presence-based cold session-targeting by friendly alias; standing
   escalation promotions.

## Decisions (resolved)

- **Surfacing → one session.** A knock or escalation prompt appears in a single
  session (`notify_one` gives this for free), never broadcast. Approve wherever it
  lands.
- **Escalation scope → ephemeral, per session-pair (final).** A capability is
  granted between two *specific* sessions (local `key#token` ↔ remote `key#token`)
  and dies when either ends. A new session on *either* side starts at the floor and
  re-requests — by design: ending a session means you're done, and you set
  capabilities fresh next time. **No durable/standing grants** (revocation-by-
  ephemerality is the whole point; terminating a session/delegate is immediate,
  total revocation). Only *admission* is inherited across sessions, never
  capability.
- **No inline `*`.** *Everything* — including full trust — runs in the capped
  delegate; the main session never ingests raw peer text. `*` is simply the widest
  possible delegate (all tools), not a bypass into your main context.
- **Ceiling told upfront.** A session's granted ceiling is advertised to the peer
  so its delegate escalates proactively instead of discovering limits by hitting
  missing tools.

### Still open

- Exact **escalation-request payload** and how a ceiling is **advertised** on the
  control plane (told-upfront needs a small handshake message + its signed `kind`).
- ~~Resume vs. always-floor~~ **Resolved: always-floor.** A reconnecting session
  re-requests; no durable per-pair grant state.
- **Delegate cost controls:** a live per-conversation delegate + replay-on-escalate
  has token/latency cost; may want a cap on concurrent delegates per session.
