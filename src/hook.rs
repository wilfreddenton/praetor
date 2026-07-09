//! The primitive: decide whether an agent may park.
//!
//! An agent may park only if its event listener is **armed**. Otherwise a
//! `block` decision is emitted, forcing the agent to re-arm before it goes idle.
//! This is the invariant that prevents a lost wakeup.
//!
//! Two properties make it safe to run on every `Stop`:
//! - **Bounded** — it blocks only while disarmed; once re-armed the next stop is
//!   allowed, so it cannot loop forever.
//! - **Fails open** — an unreachable bus allows the stop, so a dead bus can never
//!   trap the agent.

use std::time::Duration;

use serde_json::{Value, json};

/// The default listener: a long-poll that blocks until a message arrives, then
/// exits — and *that exit* is the wake.
pub fn default_listen_cmd(ca: &str, url: &str, me: &str) -> String {
    format!("curl -sN --cacert {ca} \"{url}/recv?me={me}\"")
}

/// The `block` decision handed back to Claude when no listener is armed.
pub fn block_decision(listen_cmd: &str) -> Value {
    let reason = format!(
        "Your event listener is not armed, so you would park deaf to incoming events — a lost \
         wakeup. Before stopping, re-arm it as a background task (Bash with run_in_background) \
         exactly once:\n\n    {listen_cmd}\n\n\
         When that command later returns a JSON line with \"status\":\"message\", handle the \
         message, then re-arm the same command. On \"status\":\"timeout\", just re-arm it."
    );
    json!({ "decision": "block", "reason": reason })
}

/// Ask the bus whether `me` currently has an in-flight `/recv` — i.e. a live
/// listener. Errors propagate so the caller can fail open.
pub fn check_armed(url: &str, ca_path: &str, me: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let ca_pem = std::fs::read(ca_path)?;
    let cert = reqwest::Certificate::from_pem(&ca_pem)?;
    let client = reqwest::blocking::Client::builder()
        .add_root_certificate(cert)
        .timeout(Duration::from_secs(5))
        .build()?;
    let resp: Value = client
        .get(format!("{url}/armed"))
        .query(&[("me", me)])
        .send()?
        .json()?;
    Ok(resp.get("armed").and_then(Value::as_bool).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_decision_is_a_block_and_names_the_command() {
        let cmd = default_listen_cmd("ca.pem", "https://localhost:9443", "alice");
        let d = block_decision(&cmd);
        assert_eq!(d["decision"], "block");
        let reason = d["reason"].as_str().unwrap();
        assert!(
            reason.contains(&cmd),
            "reason must embed the re-arm command"
        );
        assert!(reason.contains("lost wakeup"));
    }

    #[test]
    fn default_listen_cmd_targets_the_recv_long_poll() {
        let cmd = default_listen_cmd("/tmp/ca.pem", "https://h:1", "bob");
        assert!(cmd.contains("/recv?me=bob"));
        assert!(cmd.contains("--cacert /tmp/ca.pem"));
    }
}
