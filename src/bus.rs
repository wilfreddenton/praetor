//! The broker: one bounded FIFO per recipient.
//!
//! The bus is deliberately dumb. It routes an opaque JSON payload to a
//! recipient id and buffers while that recipient is offline. It never inspects
//! the payload, never verifies a signature, and holds no keys — so compromising
//! it lets you drop or reorder messages, but never forge one.
//!
//! Recipients are Ed25519 public keys (base64). The bus treats them as strings.

use std::collections::VecDeque;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router, http::StatusCode};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, timeout};

pub const DEFAULT_RECV_TIMEOUT_MS: u64 = 25_000;

/// A payload addressed to a recipient, stamped on arrival.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Envelope {
    pub payload: Value,
    /// Unix milliseconds, set by the bus when the message was enqueued.
    pub ts: u64,
}

/// A bounded FIFO. Bounded because an agent that never comes back online would
/// otherwise grow this without limit — the queue is the only unbounded thing in
/// the system, so it is the only thing that can leak.
struct Queue {
    items: Mutex<VecDeque<Envelope>>,
    /// Wakes a waiting `recv`. `Notify` rather than a channel so that several
    /// pending receivers can be handled without losing a permit.
    notify: Notify,
    dropped: std::sync::atomic::AtomicU64,
}

#[derive(Clone)]
pub struct Broker {
    queues: Arc<DashMap<String, Arc<Queue>>>,
    cap: usize,
}

impl Broker {
    pub fn new(cap: usize) -> Self {
        Self {
            queues: Arc::new(DashMap::new()),
            cap: cap.max(1),
        }
    }

    fn queue(&self, id: &str) -> Arc<Queue> {
        self.queues
            .entry(id.to_string())
            .or_insert_with(|| {
                Arc::new(Queue {
                    items: Mutex::new(VecDeque::new()),
                    notify: Notify::new(),
                    dropped: std::sync::atomic::AtomicU64::new(0),
                })
            })
            .clone()
    }

    /// Enqueue for `to`. Never blocks. Drops the *oldest* message when full:
    /// for a conversation, stale backlog is worth less than the newest message.
    pub async fn enqueue(&self, to: &str, payload: Value, ts: u64) {
        let q = self.queue(to);
        let mut items = q.items.lock().await;
        if items.len() >= self.cap {
            items.pop_front();
            let n = q
                .dropped
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                .saturating_add(1);
            tracing::warn!(
                to,
                cap = self.cap,
                dropped_total = n,
                "queue full; dropped oldest"
            );
        }
        items.push_back(Envelope { payload, ts });
        drop(items);
        q.notify.notify_one();
    }

    /// Wait up to `wait` for the next message for `id`, popping it.
    ///
    /// Pop-on-read: if the caller dies between here and delivering the message,
    /// it is lost. That window is a session that is already being torn down, so
    /// a lease/ack protocol would buy little. See DESIGN.md.
    pub async fn recv(&self, id: &str, wait: Duration) -> Option<Envelope> {
        let q = self.queue(id);
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Some(env) = q.items.lock().await.pop_front() {
                return Some(env);
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            // Register interest *before* re-checking, or a message enqueued in
            // between would not wake us — the classic lost-wakeup.
            let notified = q.notify.notified();
            if timeout(remaining, notified).await.is_err() {
                return None;
            }
        }
    }

    pub async fn depth(&self, id: &str) -> usize {
        self.queue(id).items.lock().await.len()
    }

    pub fn router(self) -> Router {
        Router::new()
            .route("/send", post(send))
            .route("/recv", get(recv_handler))
            .with_state(self)
    }
}

#[derive(Deserialize)]
struct SendBody {
    to: String,
    payload: Value,
}

async fn send(State(broker): State<Broker>, Json(body): Json<SendBody>) -> StatusCode {
    tracing::debug!(to = %body.to, "enqueue");
    broker
        .enqueue(&body.to, body.payload, crate::now_ms())
        .await;
    StatusCode::ACCEPTED
}

#[derive(Deserialize)]
struct RecvQuery {
    me: String,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
}

fn default_timeout() -> u64 {
    DEFAULT_RECV_TIMEOUT_MS
}

async fn recv_handler(
    State(broker): State<Broker>,
    Query(q): Query<RecvQuery>,
) -> impl IntoResponse {
    match broker
        .recv(&q.me, Duration::from_millis(q.timeout_ms))
        .await
    {
        Some(env) => Json(json!({ "status": "message", "envelope": env })),
        None => Json(json!({ "status": "timeout" })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enqueue_then_recv_returns_payload() {
        let b = Broker::new(8);
        b.enqueue("alice", json!({ "hi": 1 }), 5).await;
        let env = b.recv("alice", Duration::from_millis(50)).await.unwrap();
        assert_eq!(env.payload, json!({ "hi": 1 }));
        assert_eq!(env.ts, 5);
    }

    #[tokio::test]
    async fn recv_times_out_when_empty() {
        let b = Broker::new(8);
        assert!(b.recv("nobody", Duration::from_millis(10)).await.is_none());
    }

    #[tokio::test]
    async fn fifo_order_is_preserved() {
        let b = Broker::new(8);
        for i in 0..3 {
            b.enqueue("bob", json!(i), i).await;
        }
        for i in 0..3 {
            assert_eq!(
                b.recv("bob", Duration::from_millis(50))
                    .await
                    .unwrap()
                    .payload,
                json!(i)
            );
        }
    }

    #[tokio::test]
    async fn buffers_while_recipient_is_offline() {
        // Nothing is listening; messages must survive until someone drains them.
        let b = Broker::new(8);
        b.enqueue("bob", json!("first"), 1).await;
        b.enqueue("bob", json!("second"), 2).await;
        assert_eq!(b.depth("bob").await, 2);
        assert_eq!(
            b.recv("bob", Duration::from_millis(50))
                .await
                .unwrap()
                .payload,
            json!("first")
        );
    }

    #[tokio::test]
    async fn bounded_queue_drops_oldest() {
        let b = Broker::new(2);
        for i in 0..4 {
            b.enqueue("bob", json!(i), i).await;
        }
        assert_eq!(b.depth("bob").await, 2, "cap must hold");
        // 0 and 1 were evicted; the newest survive.
        assert_eq!(
            b.recv("bob", Duration::from_millis(50))
                .await
                .unwrap()
                .payload,
            json!(2)
        );
        assert_eq!(
            b.recv("bob", Duration::from_millis(50))
                .await
                .unwrap()
                .payload,
            json!(3)
        );
    }

    #[tokio::test]
    async fn a_waiting_recv_is_woken_by_a_later_send() {
        let b = Broker::new(8);
        let b2 = b.clone();
        let waiter = tokio::spawn(async move { b2.recv("alice", Duration::from_secs(2)).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        b.enqueue("alice", json!("wake"), 1).await;
        assert_eq!(waiter.await.unwrap().unwrap().payload, json!("wake"));
    }

    #[tokio::test]
    async fn messages_are_routed_per_recipient() {
        let b = Broker::new(8);
        b.enqueue("alice", json!("for-alice"), 1).await;
        b.enqueue("bob", json!("for-bob"), 1).await;
        assert_eq!(
            b.recv("bob", Duration::from_millis(50))
                .await
                .unwrap()
                .payload,
            json!("for-bob")
        );
        assert_eq!(
            b.recv("alice", Duration::from_millis(50))
                .await
                .unwrap()
                .payload,
            json!("for-alice")
        );
    }
}
