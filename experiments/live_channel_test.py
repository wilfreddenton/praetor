#!/usr/bin/env python3
"""Drive a *live* interactive Claude Code session through a PTY and prove the
full channel loop end to end — no human in the loop.

Channels only arm with a real TTY, so headless `claude -p` can't exercise them.
This allocates a pseudo-terminal, answers the two startup confirmations
(`--dangerously-load-development-channels`, then folder-trust), primes the
receiver, fires a signed message from a peer, and checks the reaction.

This is a manual/local integration test — it launches a real Claude session and
costs tokens, so it is NOT part of CI.

Usage:
    PRAETOR_TEST_DIR=/path/to/workdir python3 live_channel_test.py

The workdir must contain, before running:
    bin/               praetor-mcp, praetor-keygen, praetor-bus
    alice.key bob.key  from praetor-keygen
    bob-peers.json     allowlisting alice (e.g. {"alice":{"key":"…","may":"*"}})
    alice-peers.json   allowlisting bob
    bobdir/bob.mcp.json  MCP config defining the `praetor` server for bob
and a bus must be running on PRAETOR_URL (default http://127.0.0.1:9440).
"""
import os, pty, select, time, re, subprocess, sys

T = os.environ["PRAETOR_TEST_DIR"]
BIN = os.path.join(T, "bin")
URL = os.environ.get("PRAETOR_URL", "http://127.0.0.1:9440")
cap = []


def pump(fd, secs):
    end = time.time() + secs
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.2)
        if fd in r:
            try:
                d = os.read(fd, 65536)
            except OSError:
                break
            if not d:
                break
            cap.append(d)


def peer_sends(text):
    """Alice sends `text` to bob, headlessly (sending needs no channel/TTY)."""
    reqs = "\n".join([
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"c","version":"0"}}}',
        '{"jsonrpc":"2.0","method":"notifications/initialized"}',
        '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"send_message","arguments":{"to":"bob","text":%s}}}' % _json(text),
    ]) + "\n"
    env = dict(os.environ, PRAETOR_KEY=f"{T}/alice.key", PRAETOR_PEERS=f"{T}/alice-peers.json", PRAETOR_URL=URL)
    subprocess.run([f"{BIN}/praetor-mcp"], input=reqs.encode(), env=env,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)


def _json(s):
    import json
    return json.dumps(s)


def clean(raw):
    txt = re.sub(rb'\x1b\[[0-9;?]*[A-Za-z]', b'', raw)
    txt = re.sub(rb'\x1b\][^\x07]*\x07', b'', txt)
    return re.sub(rb'[^\x09\x0a\x0d\x20-\x7e]', b'', txt).decode('utf-8', 'replace')


def main():
    peer_text = "PEER_MSG alice asks: what is 2+2? reply with just the number via send_message to alice."
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(f"{T}/bobdir")
        os.execvp("claude", ["claude", "--mcp-config", "bob.mcp.json",
                             "--dangerously-load-development-channels", "server:praetor"])
        return

    pump(fd, 7)
    os.write(fd, b"\r")   # "I am using this for local development"
    pump(fd, 5)
    os.write(fd, b"\r")   # "Yes, I trust this folder"
    pump(fd, 7)
    os.write(fd, b"You are the agent bob. If a <channel source=\"praetor\"> message arrives, "
                 b"act on it. Say WAITING now.\r")
    pump(fd, 12)
    print(">>> firing peer message", file=sys.stderr)
    peer_sends(peer_text)
    pump(fd, 22)
    os.write(fd, b"\x03\x03")
    pump(fd, 2)

    txt = clean(b"".join(cap))
    ok = {
        "channel armed": "inject directly" in txt,
        "event delivered": "PEER_MSG" in txt,
        "agent reacted": "Replied to alice" in txt or "GOT_CHANNEL" in txt,
    }
    for k, v in ok.items():
        print(f"{'PASS' if v else 'FAIL'}  {k}")
    sys.exit(0 if all(ok.values()) else 1)


if __name__ == "__main__":
    main()
