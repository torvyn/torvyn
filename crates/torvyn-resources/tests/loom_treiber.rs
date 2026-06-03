//! Exhaustive interleaving check for the lock-free Treiber stack in
//! [`torvyn_resources::pool`].
//!
//! Run with:
//!
//! ```sh
//! RUSTFLAGS="--cfg loom" cargo test -p torvyn-resources --test loom_treiber
//! ```
//!
//! Under `--cfg loom` the pool's head/next atomics resolve to loom's
//! instrumented atomics, so [`loom::model`] drives the worker closures through
//! every legal thread interleaving and every spurious compare-and-swap failure.
//! The whole file is `#![cfg(loom)]`, so in normal builds it compiles to an
//! empty test binary and has zero effect on `cargo test` — loom is never even
//! fetched.
#![cfg(loom)]

use std::collections::HashSet;

use loom::sync::Arc;
use torvyn_resources::handle::PoolTier;
use torvyn_resources::pool::{TierConfig, TierPool};

/// The classic ABA trigger: while thread A is mid-`pop` (it has read the head
/// and the next pointer but not yet swung the head), thread B pops two slots
/// and pushes one back onto the top. An unprotected Treiber stack lets A's
/// stale CAS succeed and hand out an already-owned slot; the tag in the packed
/// head makes A's CAS fail and retry instead.
///
/// After both threads finish, the union of "slots still held by a thread" and
/// "slots on the free list" must equal the pool's slots exactly — each address
/// appearing once. A duplicate means aliasing (one buffer, two owners); a
/// missing slot means a leak. Either is fatal to the single-owner invariant.
#[test]
fn loom_treiber_pop_push_no_aba() {
    // The faithful pop/pop/push model has several nested CAS loops; give the
    // checker generous headroom so it explores the full interleaving space
    // rather than aborting early on the default branch budget.
    let mut model = loom::model::Builder::new();
    model.max_branches = 50_000;

    model.check(|| {
        const POOL_SIZE: u32 = 3;
        let config = TierConfig {
            tier: PoolTier::Small,
            pool_size: POOL_SIZE,
            preallocate: true,
        };
        let pool = Arc::new(TierPool::new(&config));

        // Thread A: a single pop; keeps whatever slot it gets.
        let a = {
            let pool = Arc::clone(&pool);
            loom::thread::spawn(move || pool.pop().map(|(ptr, _)| ptr.as_ptr() as usize))
        };

        // Thread B: pop, pop, push the first back — ends holding the second.
        let b = {
            let pool = Arc::clone(&pool);
            loom::thread::spawn(move || {
                let first = pool.pop();
                let second = pool.pop();
                if let Some((ptr, sz)) = first {
                    assert!(pool.push(ptr, sz), "push of a known pool slot must succeed");
                }
                second.map(|(ptr, _)| ptr.as_ptr() as usize)
            })
        };

        let held_a = a.join().expect("thread A panicked");
        let held_b = b.join().expect("thread B panicked");

        // Everything outstanding (still held by a thread) plus everything we
        // can drain off the free list must reconstruct the original slot set.
        let mut outstanding: Vec<usize> = held_a.into_iter().chain(held_b).collect();
        let mut drained = 0u32;
        while let Some((ptr, _)) = pool.pop() {
            outstanding.push(ptr.as_ptr() as usize);
            drained += 1;
            // A corrupted free list could form a cycle; cap the drain so the
            // model can never hang and let the assertions below report it.
            assert!(drained <= POOL_SIZE, "free list did not terminate (cycle)");
        }

        let distinct: HashSet<usize> = outstanding.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            outstanding.len(),
            "ABA aliasing: a buffer slot was held in two places at once",
        );
        assert_eq!(
            outstanding.len(),
            POOL_SIZE as usize,
            "a buffer slot was leaked or duplicated by the free list",
        );
    });
}
