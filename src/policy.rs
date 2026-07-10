//! Per-peer authorization policy: `peers.json`.
//!
//! A peer is a **public key** with a local petname and a **grant**. The grant is
//! the dial: `"*"` means unrestricted (handled inline, all tools — only for
//! machines you fully control), and a **capability name** means scoped: the
//! request is handled by a disposable subagent of that name, whose `tools:`
//! frontmatter limits what it can do. That frontmatter is the enforcement — the
//! subagent literally cannot call a tool it wasn't given.
//!
//! ```json
//! {
//!   "my-laptop":    { "key": "8Emom3…", "may": "*" },
//!   "build-server": { "key": "rq2AzH…", "may": "run-tests" },
//!   "some-bot":     { "key": "Zc91xK…", "may": "read-only" }
//! }
//! ```
//!
//! Here `run-tests` refers to `.claude/agents/run-tests.md`, an agent definition
//! whose `tools:` line is the capability. The **default for an unlisted peer is
//! deny-everything**, so the safe state is the fallback.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::identity::AgentId;

/// What a peer is permitted to cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    /// Unrestricted: message handled inline, every tool allowed. Reserve for
    /// peers whose private keys you hold — a compromise of any such peer becomes
    /// arbitrary tool execution here.
    All,
    /// Handled in a quarantined subagent of this name (`.claude/agents/<name>.md`),
    /// whose `tools:` frontmatter is the capability. The subagent fetches the
    /// untrusted body itself, so it never enters the main context.
    Scoped(String),
}

impl<'de> Deserialize<'de> for Grant {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        match Value::deserialize(d)? {
            Value::String(s) if s == "*" => Ok(Grant::All),
            Value::String(name) if !name.is_empty() => Ok(Grant::Scoped(name)),
            Value::String(_) => Err(D::Error::custom("capability name must not be empty")),
            _ => Err(D::Error::custom(
                "grant must be \"*\" or a capability name (agent), as a string",
            )),
        }
    }
}

impl Serialize for Grant {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Grant::All => s.serialize_str("*"),
            Grant::Scoped(name) => s.serialize_str(name),
        }
    }
}

impl Grant {
    /// The capability agent name for a scoped grant.
    pub fn capability(&self) -> Option<&str> {
        match self {
            Grant::All => None,
            Grant::Scoped(name) => Some(name),
        }
    }
}

impl Grant {
    pub fn is_unrestricted(&self) -> bool {
        matches!(self, Grant::All)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawPeer {
    key: String,
    may: Grant,
}

/// One resolved peer: its authenticated identity and what it may do.
#[derive(Debug, Clone)]
pub struct Peer {
    pub petname: String,
    pub id: AgentId,
    pub grant: Grant,
}

/// The whole allowlist. Parsing validates every key up front, so a malformed
/// `peers.json` fails loudly at startup rather than silently at message time.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    peers: Vec<Peer>,
}

impl Policy {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading peers file {}", path.display()))?;
        Self::parse(&raw)
    }

    pub fn parse(raw: &str) -> Result<Self> {
        let map: BTreeMap<String, RawPeer> =
            serde_json::from_str(raw).context("peers file is not valid policy JSON")?;
        let mut peers = Vec::with_capacity(map.len());
        for (petname, rp) in map {
            let id = AgentId::from_b64(&rp.key)
                .with_context(|| format!("peer '{petname}' has an invalid key"))?;
            peers.push(Peer {
                petname,
                id,
                grant: rp.may,
            });
        }
        // Two petnames for one key is almost certainly a mistake and makes the
        // authenticated-sender lookup ambiguous.
        for i in 0..peers.len() {
            for j in (i + 1)..peers.len() {
                if peers[i].id == peers[j].id {
                    bail!(
                        "peers '{}' and '{}' share the same key",
                        peers[i].petname,
                        peers[j].petname
                    );
                }
            }
        }
        Ok(Self { peers })
    }

    /// Look up an *authenticated* sender. `None` ⇒ not on the allowlist ⇒ drop.
    pub fn peer(&self, id: AgentId) -> Option<&Peer> {
        self.peers.iter().find(|p| p.id == id)
    }

    /// Resolve a petname to a key for outbound sends.
    pub fn resolve(&self, petname: &str) -> Result<AgentId> {
        self.peers
            .iter()
            .find(|p| p.petname == petname)
            .map(|p| p.id)
            .with_context(|| format!("unknown peer '{petname}'"))
    }

    pub fn len(&self) -> usize {
        self.peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::AgentKey;

    fn policy_with(may: &str) -> (AgentKey, Policy) {
        let k = AgentKey::generate().unwrap();
        let raw = format!(
            r#"{{ "alice": {{ "key": "{}", "may": {} }} }}"#,
            k.id().to_b64(),
            may
        );
        (k, Policy::parse(&raw).unwrap())
    }

    #[test]
    fn wildcard_parses_as_unrestricted() {
        let (k, p) = policy_with("\"*\"");
        let peer = p.peer(k.id()).unwrap();
        assert_eq!(peer.petname, "alice");
        assert!(peer.grant.is_unrestricted());
    }

    #[test]
    fn capability_name_parses_as_scoped() {
        let (k, p) = policy_with("\"run-tests\"");
        assert_eq!(
            p.peer(k.id()).unwrap().grant,
            Grant::Scoped("run-tests".into())
        );
        assert_eq!(
            p.peer(k.id()).unwrap().grant.capability(),
            Some("run-tests")
        );
    }

    #[test]
    fn empty_capability_name_is_rejected() {
        let k = AgentKey::generate().unwrap();
        let raw = format!(
            r#"{{ "alice": {{ "key": "{}", "may": "" }} }}"#,
            k.id().to_b64()
        );
        assert!(Policy::parse(&raw).is_err());
    }

    #[test]
    fn unlisted_sender_is_denied() {
        let (_k, p) = policy_with("\"*\"");
        let stranger = AgentKey::generate().unwrap();
        assert!(
            p.peer(stranger.id()).is_none(),
            "unlisted key must not resolve"
        );
    }

    #[test]
    fn non_string_grant_is_rejected() {
        let k = AgentKey::generate().unwrap();
        let raw = format!(
            r#"{{ "alice": {{ "key": "{}", "may": ["x"] }} }}"#,
            k.id().to_b64()
        );
        assert!(
            Policy::parse(&raw).is_err(),
            "an array is no longer a valid grant"
        );
    }

    #[test]
    fn invalid_key_fails_at_parse_time() {
        let raw = r#"{ "alice": { "key": "not-a-real-key", "may": "*" } }"#;
        assert!(Policy::parse(raw).is_err());
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        let k = AgentKey::generate().unwrap();
        let raw = format!(
            r#"{{ "a": {{ "key": "{0}", "may": "*" }}, "b": {{ "key": "{0}", "may": [] }} }}"#,
            k.id().to_b64()
        );
        assert!(Policy::parse(&raw).is_err());
    }

    #[test]
    fn resolve_maps_petname_to_key() {
        let (k, p) = policy_with("\"*\"");
        assert_eq!(p.resolve("alice").unwrap(), k.id());
        assert!(p.resolve("nobody").is_err());
    }

    #[test]
    fn grant_round_trips_through_json() {
        assert_eq!(serde_json::to_string(&Grant::All).unwrap(), "\"*\"");
        assert_eq!(
            serde_json::to_string(&Grant::Scoped("run-tests".into())).unwrap(),
            "\"run-tests\""
        );
    }
}
