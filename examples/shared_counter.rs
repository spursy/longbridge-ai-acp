//! The same counter as `actor_counter.rs`, but WITHOUT the actor pattern.
//!
//! Instead of a task owning the state and receiving messages, the state is
//! *shared* across tasks and protected by a synchronization primitive. Three
//! idiomatic alternatives are shown, from lightest to most general:
//!
//!   1. `Arc<AtomicI64>`     — lock-free, best for a single integer counter.
//!   2. `Arc<Mutex<i64>>`    — a lock around the state; general-purpose.
//!   3. `Arc<RwLock<i64>>`   — many concurrent readers, exclusive writers.
//!
//! All three reach `final count = 200`, just like the actor version — the
//! difference is *how* concurrency safety is achieved: shared memory + locking
//! here, vs. message passing + ownership in the actor.
//!
//! Run it:
//!   cargo run --example shared_counter

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// 1. Lock-free shared counter using an atomic integer.
async fn with_atomic() -> i64 {
    let count = Arc::new(AtomicI64::new(0));

    let c1 = Arc::clone(&count);
    let c2 = Arc::clone(&count);
    let h1 = tokio::spawn(async move {
        for _ in 0..100 {
            // No lock at all: the hardware guarantees atomicity.
            c1.fetch_add(1, Ordering::Relaxed);
        }
    });
    let h2 = tokio::spawn(async move {
        for _ in 0..100 {
            c2.fetch_add(1, Ordering::Relaxed);
        }
    });
    let _ = tokio::join!(h1, h2);

    count.load(Ordering::Relaxed)
}

/// 2. Shared counter behind a Mutex (the classic shared-memory approach).
async fn with_mutex() -> i64 {
    let count = Arc::new(Mutex::new(0i64));

    let c1 = Arc::clone(&count);
    let c2 = Arc::clone(&count);
    let h1 = tokio::spawn(async move {
        for _ in 0..100 {
            // Lock, mutate, unlock (guard dropped at end of block).
            let mut guard = c1.lock().await;
            *guard += 1;
        }
    });
    let h2 = tokio::spawn(async move {
        for _ in 0..100 {
            let mut guard = c2.lock().await;
            *guard += 1;
        }
    });
    let _ = tokio::join!(h1, h2);

    let total = *count.lock().await;
    total
}

/// 3. Shared counter behind an RwLock: cheap concurrent reads, exclusive writes.
async fn with_rwlock() -> i64 {
    let count = Arc::new(RwLock::new(0i64));

    let c1 = Arc::clone(&count);
    let c2 = Arc::clone(&count);
    let h1 = tokio::spawn(async move {
        for _ in 0..100 {
            let mut guard = c1.write().await; // writers are exclusive
            *guard += 1;
        }
    });
    let h2 = tokio::spawn(async move {
        for _ in 0..100 {
            let mut guard = c2.write().await;
            *guard += 1;
        }
    });
    let _ = tokio::join!(h1, h2);

    let total = *count.read().await; // a shared read lock
    total
}

#[tokio::main]
async fn main() {
    println!("atomic  -> final count = {}", with_atomic().await);
    println!("mutex   -> final count = {}", with_mutex().await);
    println!("rwlock  -> final count = {}", with_rwlock().await);
}
