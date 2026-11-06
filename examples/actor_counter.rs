//! Actor pattern in Rust (tokio): a counter actor that owns its state and is
//! driven purely by messages — no `Arc<Mutex<_>>`, no locks.
//!
//! The actor is a spawned task that exclusively owns `count`; callers can only
//! send `CounterMsg` into its mailbox (an `mpsc` channel). A `Get` carries a
//! `oneshot::Sender` so the caller can await the reply, turning a one-way
//! message into a request/response call.
//!
//! Run it:
//!   cargo run --example actor_counter

use tokio::sync::{mpsc, oneshot};

/// Messages the actor accepts (its "interface").
enum CounterMsg {
    Increment,
    /// Carries a one-shot reply channel so the caller can await the value.
    Get(oneshot::Sender<i64>),
}

/// The actor body: it alone owns `count`, so no lock is ever needed.
async fn counter_actor(mut mailbox: mpsc::Receiver<CounterMsg>) {
    let mut count: i64 = 0;
    while let Some(msg) = mailbox.recv().await {
        match msg {
            CounterMsg::Increment => count += 1,
            CounterMsg::Get(reply) => {
                let _ = reply.send(count);
            }
        }
    }
}

/// Cloneable handle: exposes only "send a message", never the state itself.
#[derive(Clone)]
struct CounterHandle {
    sender: mpsc::Sender<CounterMsg>,
}

impl CounterHandle {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel(32);
        tokio::spawn(counter_actor(rx));
        Self { sender: tx }
    }

    async fn increment(&self) {
        let _ = self.sender.send(CounterMsg::Increment).await;
    }

    async fn get(&self) -> i64 {
        let (tx, rx) = oneshot::channel();
        let _ = self.sender.send(CounterMsg::Get(tx)).await;
        rx.await.expect("actor dropped the reply channel")
    }
}

#[tokio::main]
async fn main() {
    let counter = CounterHandle::new();

    // Clone the handle and drive it concurrently from several tasks.
    // No lock, no data race — every mutation is serialized inside the actor.
    let c1 = counter.clone();
    let c2 = counter.clone();
    let h1 = tokio::spawn(async move {
        for _ in 0..100 {
            c1.increment().await;
        }
    });
    let h2 = tokio::spawn(async move {
        for _ in 0..100 {
            c2.increment().await;
        }
    });
    let _ = tokio::join!(h1, h2);

    println!("final count = {}", counter.get().await); // 200
}
