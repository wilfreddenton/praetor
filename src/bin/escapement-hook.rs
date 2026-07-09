//! Claude Code `Stop` hook. Refuses to let this agent park unless its event
//! listener is armed.
//!
//! Config comes from flags or the matching env var (flags win):
//!   --self / ESC_SELF   this agent's id on the bus
//!   --url  / ESC_URL    bus base url (default https://localhost:9443)
//!   --ca   / ESC_CA     path to the bus's ca.pem
//!   --listen-cmd / ESC_LISTEN_CMD   override the re-arm command shown to Claude

use std::io::Read;

use clap::Parser;
use escapement::hook::{block_decision, check_armed, default_listen_cmd};

#[derive(Parser)]
struct Config {
    #[arg(long = "self", env = "ESC_SELF")]
    me: String,
    #[arg(long, env = "ESC_URL", default_value = "https://localhost:9443")]
    url: String,
    #[arg(long, env = "ESC_CA")]
    ca: String,
    #[arg(long, env = "ESC_LISTEN_CMD")]
    listen_cmd: Option<String>,
}

fn main() {
    escapement::install_crypto();

    // Drain the hook payload on stdin; our decision only needs config.
    let mut _buf = String::new();
    let _ = std::io::stdin().read_to_string(&mut _buf);

    // A config error is a setup mistake — allow the stop rather than trap Claude.
    let Ok(cfg) = Config::try_parse() else {
        return;
    };

    match check_armed(&cfg.url, &cfg.ca, &cfg.me) {
        Ok(true) => {} // listener live -> allow the park
        Err(_) => {}   // bus unreachable -> fail open, allow the park
        Ok(false) => {
            let cmd = cfg
                .listen_cmd
                .clone()
                .unwrap_or_else(|| default_listen_cmd(&cfg.ca, &cfg.url, &cfg.me));
            println!("{}", block_decision(&cmd));
        }
    }
}
