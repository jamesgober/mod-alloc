//! Reentrancy-guard smoke test.
//!
//! v0.9.0's `GlobalAlloc` impl does no internal heap allocation in
//! its hot path (it forwards to `System` and updates atomic
//! counters), so direct re-entry cannot be triggered through
//! ordinary user code. This test instead exercises the patterns
//! that the guard exists to protect against: allocation-heavy
//! workloads with deeply nested `Drop` chains and formatting paths.
//!
//! If the reentrancy machinery were ever broken (e.g. a future
//! change to the hook itself allocates), this workload would either
//! stack-overflow or hang. Reaching the end of `main` proves the
//! invariant holds for the present implementation.

use mod_alloc::ModAlloc;

#[global_allocator]
static GLOBAL: ModAlloc = ModAlloc::new();

struct AllocOnDrop {
    payload: Vec<u8>,
}

impl Drop for AllocOnDrop {
    fn drop(&mut self) {
        // Allocate inside Drop. With the reentrancy guard in place,
        // any re-entry from inside the allocator hook would short
        // circuit instead of recursing.
        let scratch: Vec<u8> = Vec::with_capacity(self.payload.len() * 2);
        std::hint::black_box(&scratch);
    }
}

#[test]
fn allocator_handles_drop_chains_without_recursion() {
    let before = GLOBAL.snapshot();

    let mut chains: Vec<AllocOnDrop> = Vec::with_capacity(100);
    for i in 0..100 {
        chains.push(AllocOnDrop {
            payload: vec![i as u8; 32],
        });
    }

    // Mix in formatting paths, which historically can re-enter the
    // allocator in some configurations via panic infrastructure or
    // dynamic string growth.
    let mut acc = String::new();
    for i in 0..200 {
        acc.push_str(&format!("item-{i:08}-{i:08x}\n"));
    }
    assert!(!acc.is_empty());

    drop(chains);

    let after = GLOBAL.snapshot();
    assert!(
        after.alloc_count > before.alloc_count,
        "expected counter movement; got {} -> {}",
        before.alloc_count,
        after.alloc_count
    );
}
