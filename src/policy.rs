//! Per-peer authorization policy: `peers.json`.
//!
//! A peer is a **public key** with a local petname and a **grant**. The grant is
//! the dial: `"*"` means unrestricted (handled inline, all tools — only for
//! machines you fully control), and a list means scoped (handled in a disposable
//! subagent whose tools are limited to those patterns).
//!
//! ```json
//! {
//!   "my-laptop":    { "key": "8Emom3…", "may": "*" },
//!   "build-server": { "key": "rq2AzH…", "may": ["Bash(cargo test:*)", "Read"] },
//!   "some-bot":     { "key": "Zc91xK…", "may": [] }
//! }
//! ```
//!
//! The **default for an unlisted or unparseable peer is deny-everything**, so the
//! safe state is the fallback.

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
    /// Handled in a quarantined subagent limited to these tool patterns. An
    /// empty list means "may send, but may run nothing" — useful for a peer that
    /// only delivers information.
    Scoped(Vec<String>),
}

impl<'de> Deserialize<'de> for Grant {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        match Value::deserialize(d)? {
            Value::String(s) if s == "*" => Ok(Grant::All),
            Value::String(other) => Err(D::Error::custom(format!(
                "grant must be \"*\" or a list of tool patterns, got \"{other}\""
            ))),
            Value::Array(items) => {
                let mut patterns = Vec::with_capacity(items.len());
                for it in items {
                    match it {
                        Value::String(s) => patterns.push(s),
                        _ => return Err(D::Error::custom("tool patterns must be strings")),
                    }
                }
                Ok(Grant::Scoped(patterns))
            }
            _ => Err(D::Error::custom(
                "grant must be \"*\" or a list of tool patterns",
            )),
        }
    }
}

impl Serialize for Grant {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Grant::All => s.serialize_str("*"),
            Grant::Scoped(v) => v.serialize(s),
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
    fn list_parses_as_scoped() {
        let (k, p) = policy_with(r#"["Bash(cargo test:*)", "Read"]"#);
        match &p.peer(k.id()).unwrap().grant {
            Grant::Scoped(v) => assert_eq!(v, &["Bash(cargo test:*)", "Read"]),
            Grant::All => panic!("should be scoped"),
        }
    }

    #[test]
    fn empty_list_is_scoped_with_no_tools() {
        let (k, p) = policy_with("[]");
        assert_eq!(p.peer(k.id()).unwrap().grant, Grant::Scoped(vec![]));
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
    fn bad_grant_string_is_rejected() {
        let k = AgentKey::generate().unwrap();
        let raw = format!(
            r#"{{ "alice": {{ "key": "{}", "may": "everything" }} }}"#,
            k.id().to_b64()
        );
        assert!(Policy::parse(&raw).is_err());
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
            serde_json::to_string(&Grant::Scoped(vec!["Read".into()])).unwrap(),
            "[\"Read\"]"
        );
    }
}
