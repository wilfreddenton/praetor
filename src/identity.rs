//! Ed25519 identity. **The public key is the identity**; names are local petnames.
//!
//! Claiming the name "alice" gets you nothing without her key, so the sender
//! gate in the channel server checks a signature rather than a string an agent
//! typed. This is the property the channel docs demand: *"gate on the sender's
//! identity."*
//!
//! Signing covers a domain-separated, length-prefixed canonical encoding, so a
//! signature can never be replayed into a different context and no pair of
//! fields can be shifted across their boundary.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_dalek::{
    PUBLIC_KEY_LENGTH, SECRET_KEY_LENGTH, SIGNATURE_LENGTH, Signature, Signer, SigningKey,
    VerifyingKey,
};
use serde::{Deserialize, Serialize};

/// Bound into every message signature. Bumped v1→v2 when `kind` entered the
/// canonical encoding; a bump makes old and new signatures mutually unverifiable.
const DOMAIN: &[u8] = b"interlink-v2\0";

/// What a signed message *is*. A plain `Message` is the everyday case; the pairing
/// kinds are the only thing a non-peer may deliver (a knock), gated specially.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    #[default]
    Message,
    PairRequest,
    PairAccept,
}

impl MessageKind {
    fn as_str(self) -> &'static str {
        match self {
            MessageKind::Message => "message",
            MessageKind::PairRequest => "pair_request",
            MessageKind::PairAccept => "pair_accept",
        }
    }
}

/// An agent's identity: its Ed25519 public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentId(VerifyingKey);

impl AgentId {
    pub fn from_b64(s: &str) -> Result<Self> {
        let raw = B64.decode(s).context("agent id is not valid base64")?;
        let bytes: [u8; PUBLIC_KEY_LENGTH] = raw
            .try_into()
            .map_err(|_| anyhow!("agent id must be {PUBLIC_KEY_LENGTH} bytes"))?;
        Ok(Self(
            VerifyingKey::from_bytes(&bytes).context("not a valid ed25519 public key")?,
        ))
    }

    pub fn to_b64(self) -> String {
        B64.encode(self.0.as_bytes())
    }

    /// Short form for logs and `<channel>` tags. Not for authentication.
    pub fn fingerprint(self) -> String {
        self.to_b64().chars().take(8).collect()
    }

    pub fn as_verifying_key(&self) -> &VerifyingKey {
        &self.0
    }
}

/// An agent's secret key. Zeroized on drop by `ed25519-dalek`'s default features.
pub struct AgentKey(SigningKey);

impl AgentKey {
    /// Seeded straight from the OS CSPRNG.
    pub fn generate() -> Result<Self> {
        let mut secret = [0u8; SECRET_KEY_LENGTH];
        getrandom::fill(&mut secret).map_err(|e| anyhow!("OS entropy unavailable: {e}"))?;
        Ok(Self(SigningKey::from_bytes(&secret)))
    }

    pub fn from_b64(s: &str) -> Result<Self> {
        let raw = B64
            .decode(s.trim())
            .context("secret key is not valid base64")?;
        let bytes: [u8; SECRET_KEY_LENGTH] = raw
            .try_into()
            .map_err(|_| anyhow!("secret key must be {SECRET_KEY_LENGTH} bytes"))?;
        Ok(Self(SigningKey::from_bytes(&bytes)))
    }

    pub fn to_b64(&self) -> String {
        B64.encode(self.0.to_bytes())
    }

    pub fn id(&self) -> AgentId {
        AgentId(self.0.verifying_key())
    }

    /// Sign a plain message to `to`. `ts` and `msg_id` are covered, giving replay
    /// protection when paired with the receiver's dedupe set.
    pub fn sign(&self, to: AgentId, text: &str, ts: u64, msg_id: &str) -> SignedMessage {
        self.sign_as(to, text, ts, msg_id, MessageKind::Message)
    }

    /// Sign a message of a specific `kind` (plain, or a pairing knock/accept).
    pub fn sign_as(
        &self,
        to: AgentId,
        text: &str,
        ts: u64,
        msg_id: &str,
        kind: MessageKind,
    ) -> SignedMessage {
        let bytes = canonical(self.id(), to, ts, msg_id, text, kind);
        let sig: Signature = self.0.sign(&bytes);
        SignedMessage {
            from: self.id().to_b64(),
            to: to.to_b64(),
            text: text.to_string(),
            ts,
            msg_id: msg_id.to_string(),
            kind,
            sig: B64.encode(sig.to_bytes()),
        }
    }

    /// Sign a presence announcement: "this key is online, calling itself `name`".
    pub fn announce(&self, name: &str, ts: u64) -> Announcement {
        let sig: Signature = self.0.sign(&announce_canonical(self.id(), name, ts));
        Announcement {
            pubkey: self.id().to_b64(),
            name: name.to_string(),
            ts,
            sig: B64.encode(sig.to_bytes()),
        }
    }
}

/// What travels over the bus. The bus treats it as an opaque payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedMessage {
    pub from: String,
    pub to: String,
    pub text: String,
    pub ts: u64,
    pub msg_id: String,
    #[serde(default)]
    pub kind: MessageKind,
    pub sig: String,
}

impl SignedMessage {
    /// Verify the signature and return the *authenticated* sender.
    ///
    /// Uses `verify_strict`, which rejects small-order public keys and
    /// non-canonical signature encodings.
    pub fn verify(&self) -> Result<AgentId> {
        let from = AgentId::from_b64(&self.from)?;
        let to = AgentId::from_b64(&self.to)?;

        let raw = B64
            .decode(&self.sig)
            .context("signature is not valid base64")?;
        let sig_bytes: [u8; SIGNATURE_LENGTH] = raw
            .try_into()
            .map_err(|_| anyhow!("signature must be {SIGNATURE_LENGTH} bytes"))?;
        let sig = Signature::from_bytes(&sig_bytes);

        let bytes = canonical(from, to, self.ts, &self.msg_id, &self.text, self.kind);
        from.as_verifying_key()
            .verify_strict(&bytes, &sig)
            .map_err(|_| {
                anyhow!(
                    "signature does not verify for sender {}",
                    from.fingerprint()
                )
            })?;
        Ok(from)
    }
}

/// Domain-separated, length-prefixed. Length prefixes stop an attacker shifting
/// bytes across the `msg_id`/`text` boundary to forge a different message under
/// the same signature.
fn canonical(
    from: AgentId,
    to: AgentId,
    ts: u64,
    msg_id: &str,
    text: &str,
    kind: MessageKind,
) -> Vec<u8> {
    let k = kind.as_str();
    let mut b = Vec::with_capacity(DOMAIN.len() + 88 + k.len() + msg_id.len() + text.len());
    b.extend_from_slice(DOMAIN);
    b.extend_from_slice(from.as_verifying_key().as_bytes());
    b.extend_from_slice(to.as_verifying_key().as_bytes());
    b.extend_from_slice(&ts.to_le_bytes());
    b.extend_from_slice(&(k.len() as u32).to_le_bytes());
    b.extend_from_slice(k.as_bytes());
    b.extend_from_slice(&(msg_id.len() as u32).to_le_bytes());
    b.extend_from_slice(msg_id.as_bytes());
    b.extend_from_slice(&(text.len() as u32).to_le_bytes());
    b.extend_from_slice(text.as_bytes());
    b
}

/// Bound into presence announcements. Separate from the message `DOMAIN` so the
/// two version independently — a message-format change need not reissue the
/// announcement format, and vice versa.
const ANNOUNCE_DOMAIN: &[u8] = b"interlink-announce-v1\0";

/// A signed presence announcement, published to the bus roster. The `name` is a
/// self-claim; identity is the key, so a peer [`verify`](Announcement::verify)s
/// before ever trusting the name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Announcement {
    pub pubkey: String,
    pub name: String,
    pub ts: u64,
    pub sig: String,
}

impl Announcement {
    /// Verify the self-signature; returns the authenticated key on success.
    pub fn verify(&self) -> Result<AgentId> {
        let id = AgentId::from_b64(&self.pubkey)?;
        let raw = B64
            .decode(&self.sig)
            .context("announcement signature is not valid base64")?;
        let sig_bytes: [u8; SIGNATURE_LENGTH] = raw
            .try_into()
            .map_err(|_| anyhow!("signature must be {SIGNATURE_LENGTH} bytes"))?;
        let sig = Signature::from_bytes(&sig_bytes);
        id.as_verifying_key()
            .verify_strict(&announce_canonical(id, &self.name, self.ts), &sig)
            .map_err(|_| anyhow!("announcement does not verify for {}", id.fingerprint()))?;
        Ok(id)
    }
}

fn announce_canonical(pubkey: AgentId, name: &str, ts: u64) -> Vec<u8> {
    let mut b = Vec::with_capacity(ANNOUNCE_DOMAIN.len() + 40 + name.len());
    b.extend_from_slice(ANNOUNCE_DOMAIN);
    b.extend_from_slice(pubkey.as_verifying_key().as_bytes());
    b.extend_from_slice(&(name.len() as u32).to_le_bytes());
    b.extend_from_slice(name.as_bytes());
    b.extend_from_slice(&ts.to_le_bytes());
    b
}

/// Reject messages whose timestamp is too far from now, bounding replay windows.
pub fn check_freshness(ts: u64, now: u64, max_skew_ms: u64) -> Result<()> {
    let delta = now.abs_diff(ts);
    if delta > max_skew_ms {
        bail!("message timestamp is {delta}ms from now (max {max_skew_ms}ms)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> AgentKey {
        AgentKey::generate().unwrap()
    }

    #[test]
    fn sign_then_verify_returns_the_sender() {
        let (alice, bob) = (key(), key());
        let msg = alice.sign(bob.id(), "hello", 1234, "m1");
        assert_eq!(msg.verify().unwrap(), alice.id());
    }

    #[test]
    fn tampered_text_fails() {
        let (alice, bob) = (key(), key());
        let mut msg = alice.sign(bob.id(), "transfer 1", 1234, "m1");
        msg.text = "transfer 1000".into();
        assert!(msg.verify().is_err());
    }

    #[test]
    fn tampered_recipient_fails() {
        let (alice, bob, eve) = (key(), key(), key());
        let mut msg = alice.sign(bob.id(), "hi", 1, "m1");
        msg.to = eve.id().to_b64();
        assert!(msg.verify().is_err());
    }

    #[test]
    fn forged_sender_fails() {
        // Eve signs, then claims to be Alice.
        let (alice, bob, eve) = (key(), key(), key());
        let mut msg = eve.sign(bob.id(), "hi", 1, "m1");
        msg.from = alice.id().to_b64();
        assert!(msg.verify().is_err());
    }

    #[test]
    fn ts_and_msg_id_are_covered() {
        let (alice, bob) = (key(), key());
        let orig = alice.sign(bob.id(), "hi", 1, "m1");
        for mut m in [orig.clone(), orig.clone()] {
            m.ts = 2;
            assert!(m.verify().is_err(), "ts must be signed");
        }
        let mut m = orig;
        m.msg_id = "m2".into();
        assert!(m.verify().is_err(), "msg_id must be signed");
    }

    #[test]
    fn length_prefixes_stop_boundary_shifting() {
        // ("ab", "c") and ("a", "bc") must not produce the same signed bytes.
        let (alice, bob) = (key(), key());
        let a = canonical(alice.id(), bob.id(), 1, "ab", "c", MessageKind::Message);
        let b = canonical(alice.id(), bob.id(), 1, "a", "bc", MessageKind::Message);
        assert_ne!(a, b);
    }

    #[test]
    fn domain_separation_is_bound_in() {
        let (alice, bob) = (key(), key());
        let bytes = canonical(alice.id(), bob.id(), 1, "m1", "hi", MessageKind::Message);
        assert!(bytes.starts_with(DOMAIN));
    }

    #[test]
    fn id_b64_round_trips() {
        let alice = key();
        let id = alice.id();
        assert_eq!(AgentId::from_b64(&id.to_b64()).unwrap(), id);
        assert_eq!(id.fingerprint().len(), 8);
    }

    #[test]
    fn secret_key_b64_round_trips() {
        let alice = key();
        let restored = AgentKey::from_b64(&alice.to_b64()).unwrap();
        assert_eq!(restored.id(), alice.id());
    }

    #[test]
    fn freshness_bounds_replay_window() {
        assert!(check_freshness(1_000, 1_500, 1_000).is_ok());
        assert!(check_freshness(1_000, 5_000, 1_000).is_err());
        assert!(
            check_freshness(5_000, 1_000, 1_000).is_err(),
            "future ts too"
        );
    }

    #[test]
    fn announcement_round_trips_and_rejects_tampering() {
        let alice = key();
        let a = alice.announce("alice-laptop", 1234);
        assert_eq!(a.verify().unwrap(), alice.id());

        let mut tampered_name = a.clone();
        tampered_name.name = "eve-laptop".into();
        assert!(tampered_name.verify().is_err(), "name is signed");

        let mut forged_key = a;
        forged_key.pubkey = key().id().to_b64();
        assert!(
            forged_key.verify().is_err(),
            "can't reattribute to another key"
        );
    }

    #[test]
    fn kind_is_covered_by_signature() {
        let (alice, bob) = (key(), key());
        let mut m = alice.sign_as(bob.id(), "hi", 1, "m1", MessageKind::Message);
        assert!(m.verify().is_ok());
        // A message can't be re-typed into a pairing knock under the same signature.
        m.kind = MessageKind::PairRequest;
        assert!(m.verify().is_err(), "kind must be signed");
    }
}
