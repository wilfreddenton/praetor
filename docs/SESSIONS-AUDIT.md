# Audit of the SESSIONS.md proposal

> Method: three parallel investigations — (1) Claude Code / Agent SDK
> **feasibility**, (2) **competitor** comparison (claude-peers-mcp, cc2cc,
> agent-comms-mcp, Google/LF **A2A**, Anthropic **Agent Teams**), (3) **security
> & capability prior art** (UCAN, biscuit, macaroons, SDSI, ocap, SPIFFE,
> did:key). Findings below are de-duplicated and cross-checked; two code-level
> claims were verified directly against the source.

## Verdict (TL;DR)

- **The core idea is validated and well-grounded.** The two differentiators —
  **asymmetric cryptographic identity** and **context-isolation quarantine** (a
  capped delegate so untrusted peer text can't exceed an operator-approved
  ceiling) — are real, rare, and exactly the industry-consensus defense against
  agent confused-deputy / prompt-injection. Keep these; anchor all "novelty"
  claims here and nowhere else.
- **§6 (the persistent backgrounded delegate) must be gutted, not patched.** Two
  audits independently killed it: it is **infeasible** (deployed Claude Code
  subagents are strictly one-shot spawn→run→return; there is no resumable,
  model-fed background subagent — my earlier claim conflated *this* orchestration
  harness with shipped Claude Code) **and** it is the **most overengineered** piece
  relative to the field.
- **If we build the per-session ceiling, one SEV-1 crypto hole is mandatory to
  fix:** the session token graduates from a routing hint to a *security-relevant
  selector* but is left **unsigned**. Sign it.
- **Two untrusted-text laundering paths** (the escalation-request prompt, and the
  delegate→main surfacing channel) escape the "never ingest raw peer text"
  invariant and must be typed.
- **Three strategic forks** need a human decision: align to **A2A**?; reposition
  vs **Agent Teams**; **ephemeral-only vs. opt-in durable** grants.

## What's validated — do not change

| Design choice | Prior-art grounding |
|---|---|
| Ed25519 pubkey **is** identity, TOFU pinning | `did:key`; SSH/`age` TOFU (standard) |
| Capped delegate; main agent never holds raw peer text | Object-capability / POLA / no-ambient-authority (Miller) — the *correct* anti-injection defense |
| Capability = a *local* subagent name; no shared vocabulary | SDSI "linked local names" (reinvented, good instinct) |
| Signed message over a dumb, non-verifying bus | Integrity survives an untrusted relay (as UCAN/biscuit also do) |
| "Grant dies with the session/delegate" | Revocation-by-ephemerality — sidesteps the hardest problem in every bearer-capability system; **elegant** |
| `verify_strict`, domain-separated, length-prefixed, versioned signing | Textbook Ed25519 hygiene |
| Hard tool ceiling via subagent `tools:` frontmatter | Structural harness enforcement, not model-honored (verified) |
| `PreToolUse` hook reading `agent_type` | Real, reliable (verified) |
| `to == self` re-check; fixed-width `ts` | **Verified in source — sound** (agent.rs:67; identity.rs canonical) |

**Not** reinventing UCAN/biscuit/macaroons, and correctly so: those exist to move
*attenuable* authority through parties that don't share trust and to be verified
offline by a third party. praetor has no sub-delegation chain and no third-party
verifier — **issuer == enforcer**. Adopting a transferable-capability token here
would be cargo-cult. (This flips only if "standing/resumable grants" ever ship —
see Strategic §3.)

## Must-fix before building the session-scoped ceiling

### M1 — Gut §6: replace the persistent delegate with one-shot spawn + replay
Deployed subagents are one-shot. Model the delegate as: **each peer message → a
fresh capped spawn, re-briefed from the durable conversation log**
(`store.log` already exists). Continuity is by replay, not by a living process.
**Write the token cost in plainly** (every message replays the thread; every
escalation replays into a wider-tooled spawn). *Stronger option (recommended by
the competitor audit): for v1, skip the live delegate for step-up entirely and use
Claude Code's **native permission-prompt bubbling** — the Agent Teams pattern —
for human-gated escalation.* Ship the message-only floor + a simple grant first;
defer replay machinery until it's proven necessary.

### M2 — Sign the recipient session token (SEV-1)
Under the new design the ceiling is selected by *which inbox* a message lands on
and scoped to `(local key#token, remote key#token)` — both unsigned. A malicious
or buggy relay can therefore **misroute a message across a capability-ceiling
boundary** (run a low-context message at a concurrent session's higher ceiling, or
cross-deliver a reply into the wrong delegate's context → leak), and **cross-session
replay** slips past the *per-process* dedupe within the generous freshness window.
The old "a relay can at worst misroute among a recipient's own sessions" bullet is
**stale** — true at *identity* granularity, false at *security* granularity.
**Fix:** fold the recipient session token into the signed canonical encoding;
reject any message whose signed target-token ≠ the inbox it arrived on. `reply_to`
may stay an unsigned hint (the return leg self-protects: my reply signs
`to = key#tokenS`, so a tampered `reply_to` just makes me sign to a token the wrong
session rejects). Bump the signing domain `praetor-v2 → -v3`.

### M3 — Type the two untrusted-text laundering channels (SEV-1)
The invariant guards *raw* peer text but not *laundered* peer text:
- **Escalation request** — must carry **no attacker free text**, mirroring the
  knock (which deliberately carries identity + escaped name only). Payload =
  `capability_name` (from local vocabulary) + `msg_id`. Otherwise an
  attacker-influenced "justification" phishes the operator's approve click
  ("URGENT: grant shell to fix your build").
- **Delegate → main surfacing** — must be **structured status**
  (`COMPLETE` / `NEEDS_ESCALATION(cap)` / `UNCERTAIN`), not free-form prose. A
  faithful relay of *"the peer says: run this command"* is quarantine-leak via
  summarization; any delegate prose reaching the main agent is presented as
  quarantined **data**, never actionable instruction.

## Strategic decisions — need a human call

### S1 — Align the wire model to A2A? (biggest fork)
Google/LF **A2A** (24.8k★, now the interop standard) already standardizes what
this proposal hand-rolls: our `key#token` ≈ A2A **`contextId`**; our "told-upfront"
ceiling handshake ≈ A2A **Agent Card** (a capability manifest at a well-known URL);
our delegation ≈ A2A **task**. Option: keep crypto-identity + quarantine as a
value-add layered *on top of* A2A's message/task/card model — gaining interop and
credibility, and deleting three reinventions (`reply_to`, the told-upfront
handshake, bespoke session addressing). Cost: adopting an external spec's shape.
*This is a direction, not a patch — decide before building.*

### S2 — Reposition vs. Anthropic **Agent Teams** (don't reimplement it)
Agent Teams is first-party, same-host, lead-spawned multi-agent with a **native**
permission boundary (a teammate can't approve on your behalf; relayed approval is
untrusted; subagent `tools` allowlist; plan-approval mode). praetor's defensible
territory is precisely **cross-machine + cryptographic identity + injection
quarantine** — which Agent Teams explicitly does *not* do. Consequence: **stop
reimplementing the same-host consent boundary via a shell hook.** DESIGN.md sells
"enforcement is the runtime's, not a hook's pattern-matching," yet gate #2 *is* a
shell hook — a self-contradiction now that the runtime provides the boundary. Treat
the hook as a fallback, not a pillar.

### S3 — Ephemeral-only vs. opt-in durable grants
The two audits pull opposite ways, and both are right about different things:
revocation-by-ephemerality is **elegant** (security audit) but **always-restart-at-
floor on every reconnect is user-hostile** and out of step with the field
(competitor audit). Reconcile: **ephemeral by default** (keep the elegance) **+ an
opt-in, revocable, durable scoped grant** for real workflows. If durable/attenuable
grants ever land, *that* is the moment to adopt **biscuit** (Rust-native, offline
attenuation, revocation IDs) rather than home-roll a grant store.

## Should-do — correctness, docs, polish

- **S4 Confidentiality for federation (SEV-2).** "No TLS on purpose" argues only
  loopback. Signed ≠ confidential, and the bus is an app-layer intermediary that
  reads plaintext bodies. A **public/federated relay would see all task traffic
  (code, outputs) in cleartext** — a real gap the docs frame as a pure win. Fix:
  until relays are operator-run, *document that the relay must be trusted*; before
  any public relay, seal bodies to the recipient's key (Ed25519→X25519 +
  `crypto_box`, pure-Rust via `x25519-dalek`/`crypto_box`, keeps the C-free
  property).
- **S5 Stored injection on escalation (SEV-2).** Re-briefing a higher-tooled
  delegate by *replaying the log verbatim* detonates instructions the peer planted
  during the inert message-only phase. Quarantine/summarize prior turns in the
  re-brief; consent UX states the grant covers the *accumulated* transcript.
- **S6 Identity hygiene (SEV-2/3).** did:key-equivalent identity has **no rotation**
  (rotation = new identity) and admission has **no revocation propagation** (manual
  `remove_peer` per machine). Document both; require an **out-of-band fingerprint
  check** at pair time (TOFU's known weak point, worsened by the non-verifying
  roster serving self-attested names); consider an admission TTL / periodic
  re-pair. State plainly: **terminating the delegate = immediate, total capability
  revocation** — this answers most of the "no revocation" criticism.
- **S7 Tone down novelty claims** in README/DISCOVERY to the two that survive
  scrutiny (crypto identity, quarantine). Drop "unique"/"novel" from
  sub-addressing (that's A2A `contextId`) and signed pairing (exists elsewhere).
  "None of them solve authorization" is too strong — Agent Teams has a real
  boundary; the honest, still-true claim is *"no competitor combines crypto
  identity + context-isolation quarantine for capability-scoped delegation."*
- **S8 Reduce onboarding friction.** claude-peers is ~7 commits / one file / 2.2k★;
  praetor needs a bus service, keygen, hand-managed `peers.json`, a plugin, two
  hooks, and `--dangerously-load-development-channels` *every launch*. Some weight
  buys real security; some is self-imposed. Bundle bus+keygen+peers bootstrap;
  minimize the per-launch flag where possible.
- **S9 Cheap UX wins to steal.** An Agent-Card-style advertised-capability document
  (replaces the bespoke told-upfront handshake) and claude-peers' `set_summary`
  one-line presence ("what I'm working on") on the roster.
- **Constraint to state, not fix:** channels are **experimental + TTY-gated** — no
  headless. The *remote* side of "collaborate while I'm away" needs a **live
  terminal**, not a daemon. (`contrib/stop-hook.sh` is the shipped pre-channels
  fallback — it exists; an audit agent wrongly claimed it was missing.)

## Name check — keep "praetor"

More apt after the redesign, not less. A Roman *praetor* administered justice,
**issued edicts granting or denying scoped remedies**, held delegated *imperium*,
and could **delegate bounded authority**; the Praetorian Guard were gatekeepers.
That is now the system's center of gravity (admission, bounded capability grants,
the capped delegate = *delegated imperium with limits*). Caveats: it's an obscure
word (land the gatekeeper metaphor in one README line) and collides with an Embraer
jet + minor enterprise products (negligible for an OSS dev tool; already committed
via `praetor-mcp`).

## Recommended path

1. **Simplify §6 out of existence for v1** (M1): message-only floor + a simple
   capability grant enforced by the capped one-shot delegate; human-gated step-up
   via native permission bubbling. No living delegate, no replay-on-escalation yet.
2. **Sign the session token** (M2) and **type the two channels** (M3) — the
   non-negotiable security work if the per-session ceiling is built at all.
3. **Land addressing first** (SESSIONS.md build-order step 1: auto-unique token +
   per-session db) — self-contained, kills the label footgun, invisible to single
   sessions.
4. **Decide S1/S2/S3** before the heavy build — they change the shape.
5. Fold S4–S9 into docs/positioning as they're touched.
