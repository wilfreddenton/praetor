# praetor-mcp (npm wrapper)

Delivers the pure-Rust [`praetor`](https://github.com/wilfreddenton/praetor)
MCP server binary via npm, so it can be run with `npx` like any other MCP server:

```json
{ "mcpServers": { "praetor": {
  "command": "npx",
  "args": ["-y", "praetor-mcp"],
  "env": {
    "PRAETOR_KEY": "…", "PRAETOR_PEERS": "…",
    "PRAETOR_URL": "http://127.0.0.1:9440", "PRAETOR_AGENT_DB": "…"
  }
} } }
```

There is no JavaScript reimplementation — this is a native binary distributed
through npm (the same pattern as esbuild, Biome, and SWC). On install, a
`postinstall` step downloads the prebuilt static binary for your platform from
the matching GitHub Release; the `bin` shim then execs it.

Prefer not to use npm? `cargo install --git https://github.com/wilfreddenton/praetor`
gives you the same binary.

## Releasing (maintainer notes)

The npm version and the git tag must match — the `postinstall` downloads assets
from `releases/download/v<version>/`:

1. Bump the crate version (`Cargo.toml`) and this `package.json` in lockstep.
2. `git tag v<version> && git push --tags` → `release.yml` builds every target
   and attaches `praetor-mcp-<target>` assets to the release.
3. Once the release assets exist: `cd npm && npm publish`.
