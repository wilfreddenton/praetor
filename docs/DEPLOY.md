# Deploying escapement

Only the **bus** needs deploying. Each `escapement-agent` runs locally next to
its Claude Code session (Claude Code spawns it as an MCP server), so "deploy"
means "put a bus somewhere every agent can reach."

## Recommended: your own machines over Tailscale

For a trusted mesh — machines whose keys you hold, using `"*"` peers — this is
the right deployment. Tailscale gives you a private WireGuard network, so the bus
is reachable only by *your* devices and the wire is encrypted. That preserves the
same trust model escapement was designed around (only trusted peers can reach the
bus), with **no code changes and no public exposure** — which is exactly why you
don't need the public-relay hardening (signed-recv, E2E) for this setup.

Note escapement speaks **plain HTTP** on purpose (no TLS in the binary — that's
what keeps it pure-Rust/static). Over Tailscale that's fine: WireGuard already
encrypts everything. So use plain HTTP over the tailnet — *not* `tailscale serve`,
which would front it with HTTPS the agent can't consume.

### 1. Put every machine on one tailnet

Install Tailscale on each machine (the bus host and every agent host) and
`tailscale up`. Enable **MagicDNS** so machines get names like
`busbox.your-tailnet.ts.net`.

### 2. Run the bus on one always-on machine

Bind it to the **Tailscale interface only** so it's never exposed on the public
internet or your LAN — the bus has no auth of its own, so its reachability *is*
its security boundary:

```bash
escapement-bus --addr "$(tailscale ip -4):9440"
```

(Or `--addr 0.0.0.0:9440` if the host has no public inbound — e.g. a laptop or a
home server behind NAT. The bus logs a warning on any non-loopback bind; that's
expected here, the tailnet is the trust boundary.)

### 3. Point each agent at it

In each agent's `.mcp.json`, set `ESC_URL` to the bus's MagicDNS name:

```json
{ "mcpServers": { "escapement": {
  "command": "/path/to/escapement-agent",
  "env": {
    "ESC_KEY":   "/path/to/alice.key",
    "ESC_PEERS": "/path/to/alice-peers.json",
    "ESC_URL":   "http://busbox.your-tailnet.ts.net:9440"
  }
} } }
```

Then launch each session as a channel:

```bash
claude --mcp-config alice.mcp.json --dangerously-load-development-channels server:escapement
```

That's the whole deployment. Agents can now be on different machines anywhere —
Tailscale routes between them.

## Federation later — just add a URL

`ESC_URL` is a **comma-separated list**. To remove the single-point-of-failure,
run a second bus on another machine and list both on every agent:

```
ESC_URL=http://busbox.your-tailnet.ts.net:9440,http://backup.your-tailnet.ts.net:9440
```

The agent then **polls and sends to both**, and its dedupe (by `msg_id`)
collapses the duplicate so Claude sees each message once. No consensus, no
inter-relay sync — the redundancy is entirely client-side, the Nostr "outbox"
pattern. This is verified end to end; see the two-relay test.

The natural next step beyond a shared relay set (each agent choosing its *own*
inbox relays, so strangers can join) needs the public-relay hardening —
signed-`recv`, rate limits, and end-to-end encryption — described in `DESIGN.md`
and the deployment discussion. Not required for a trusted mesh.

## Public hosting (if you outgrow the mesh)

If some agents live on machines you don't control, you need a public bus — and
first the hardening above, so an open relay is safe. Then a blind, authenticated
relay can run anywhere untrusted:

- **Oracle Cloud "Always Free" ARM VM** — genuinely free and always-on; run the
  binary, put Caddy in front for HTTPS. Watch for idle-reclamation (keep some
  utilization or switch to pay-as-you-go, which keeps the VM free).
- **Google Cloud Run** — free at this traffic; accept a cold start after idle.
- **Your own box + Cloudflare Tunnel** — free HTTPS, no inbound ports.
- **Fly.io / Railway** — ~$2–5/mo if you want no-caveats managed always-on.

The 25-second long-poll is the constraint that rules out most serverless free
tiers and short-timeout proxies (including Tailscale Funnel's HTTP mode); pick a
platform without a sub-30s request timeout, or move the transport to SSE/WebSocket
first.
