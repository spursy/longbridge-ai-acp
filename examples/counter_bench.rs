//! A rough, same-workload benchmark comparing how the counter is implemented:
//!
//!   * `atomic` — `Arc<AtomicI64>`, lock-free shared memory.
//!   * `mutex`  — `Arc<Mutex<i64>>`, shared memory behind a lock.
//!   * `actor`  — a task owning the state, driven by an `mpsc` mailbox.
//!
//! Each variant runs the SAME workload: `TASKS` concurrent tasks, each doing
//! `ITERS` increments, so the final count must be `TASKS * ITERS` every time.
//! We print wall-clock time and throughput (increments/second).
//!
//! This is a micro-benchmark, not science: numbers vary by machine, tokio
//! flavor, and contention. It exists to make the *relative* cost visible —
//! message passing adds per-op overhead that raw atomics do not.
//!
//! Run it (release mode matters a lot here):
//!   cargo run --release --example counter_bench

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot, Mutex};

const TASKS: i64 = 8;
const ITERS: i64 = 100_000;
const EXPECTED: i64 = TASKS * ITERS;

/// Lock-free shared memory.
async fn bench_atomic() -> (i64, std::time::Duration) {
    let count = Arc::new(AtomicI64::new(0));
    let start = Instant::now();

    let mut handles = Vec::new();
    for _ in 0..TASKS {
        let c = Arc::clone(&count);
        handles.push(tokio::spawn(async move {
            for _ in 0..ITERS {
                c.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }

    (count.load(Ordering::Relaxed), start.elapsed())
}

/// Shared memory behind an async Mutex.
async fn bench_mutex() -> (i64, std::time::Duration) {
    let count = Arc::new(Mutex::new(0i64));
    let start = Instant::now();

    let mut handles = Vec::new();
    for _ in 0..TASKS {
        let c = Arc::clone(&count);
        handles.push(tokio::spawn(async move {
            for _ in 0..ITERS {
                let mut g = c.lock().await;
                *g += 1;
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }

    let total = *count.lock().await;
    (total, start.elapsed())
}

/// Actor: a task owns the state; every increment is a message.
async fn bench_actor() -> (i64, std::time::Duration) {
    enum Msg {
        Increment,
        Get(oneshot::Sender<i64>),
    }

    let (tx, mut rx) = mpsc::channel::<Msg>(1024);
    let actor = tokio::spawn(async move {
        let mut count: i64 = 0;
        while let Some(msg) = rx.recv().await {
            match msg {
                Msg::Increment => count += 1,
                Msg::Get(reply) => {
                    let _ = reply.send(count);
                }
            }
        }
    });

    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..TASKS {
        let tx = tx.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..ITERS {
                let _ = tx.send(Msg::Increment).await;
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }

    // Read the final value through the same mailbox, so all queued increments
    // are guaranteed to have been applied before we observe the total.
    let (reply_tx, reply_rx) = oneshot::channel();
    let _ = tx.send(Msg::Get(reply_tx)).await;
    let total = reply_rx.await.expect("actor reply");
    let elapsed = start.elapsed();

    drop(tx); // close the mailbox so the actor task can finish
    let _ = actor.await;

    (total, elapsed)
}

fn report(name: &str, total: i64, dur: std::time::Duration) {
    assert_eq!(total, EXPECTED, "{name}: wrong total (data race?)");
    let secs = dur.as_secs_f64();
    let per_sec = f64::from(u32::try_from(EXPECTED).unwrap_or(u32::MAX)) / secs;
    println!(
        "{name:<7} total={total}  time={:>8.3?}  throughput={:>12.0} ops/s",
        dur, per_sec
    );
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!(
        "workload: {TASKS} tasks x {ITERS} increments = {EXPECTED} ops (multi-thread runtime)\n"
    );

    // Warm up the runtime/allocator so the first result isn't unfairly slow.
    let _ = bench_atomic().await;

    let (t, d) = bench_atomic().await;
    report("atomic", t, d);
    let (t, d) = bench_mutex().await;
    report("mutex", t, d);
    let (t, d) = bench_actor().await;
    report("actor", t, d);
}
