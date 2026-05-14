//! # mod-alloc
//!
//! Allocation profiling for Rust. Tracks allocation counts, total
//! bytes, peak resident memory, and current resident memory by
//! wrapping the system allocator via [`GlobalAlloc`].
//!
//! Designed as a lean replacement for `dhat` with MSRV 1.75 and
//! zero external dependencies on the hot path.
//!
//! ## Installing as the global allocator
//!
//! ```no_run
//! use mod_alloc::{ModAlloc, Profiler};
//!
//! #[global_allocator]
//! static GLOBAL: ModAlloc = ModAlloc::new();
//!
//! fn main() {
//!     let p = Profiler::start();
//!
//!     let v: Vec<u64> = (0..1000).collect();
//!     drop(v);
//!
//!     let stats = p.stop();
//!     println!("Allocations: {}", stats.alloc_count);
//!     println!("Total bytes: {}", stats.total_bytes);
//!     println!("Peak bytes (absolute): {}", stats.peak_bytes);
//! }
//! ```
//!
//! ## Counter semantics
//!
//! The four Tier 1 counters track allocator activity since the
//! installed [`ModAlloc`] began counting (or since the last
//! [`ModAlloc::reset`] call):
//!
//! | Counter         | Updated on `alloc`            | Updated on `dealloc` |
//! |-----------------|-------------------------------|----------------------|
//! | `alloc_count`   | `+= 1`                        | (unchanged)          |
//! | `total_bytes`   | `+= size`                     | (unchanged)          |
//! | `current_bytes` | `+= size`                     | `-= size`            |
//! | `peak_bytes`    | high-water mark of `current`  | (unchanged)          |
//!
//! `realloc` is counted as one allocation event. `total_bytes`
//! increases by the growth delta on a growing realloc and is
//! unchanged on a shrinking realloc.
//!
//! ## Status
//!
//! v0.9.0 ships Tier 1 (counters) only. The `backtraces` and
//! `dhat-compat` cargo features are defined for forward
//! compatibility but compile as no-ops; Tier 2 (inline backtrace
//! capture) lands in v0.9.1 and Tier 3 (DHAT-compatible JSON
//! output) lands in v0.9.3.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

// Process-wide handle to the installed `ModAlloc`. Populated lazily
// on the first non-reentrant alloc call. `Profiler` reads from this
// to locate the canonical counters without requiring an explicit
// registration call from the user.
static GLOBAL_HANDLE: AtomicPtr<ModAlloc> = AtomicPtr::new(ptr::null_mut());

thread_local! {
    // Reentrancy flag. Set while inside the tracking path; if any
    // allocation occurs while set, the recursive call bypasses
    // tracking and forwards directly to the System allocator.
    //
    // `const` initialization (stable since 1.59) avoids any lazy
    // construction allocation inside the TLS access path.
    static IN_ALLOC: Cell<bool> = const { Cell::new(false) };
}

// RAII guard for the reentrancy flag. `enter` returns `None` if the
// current thread is already inside a tracked allocation (caller
// must skip counter updates) or if TLS is unavailable (e.g. during
// thread teardown). The guard clears the flag on drop.
struct ReentryGuard;

impl ReentryGuard {
    fn enter() -> Option<Self> {
        IN_ALLOC
            .try_with(|flag| {
                if flag.get() {
                    None
                } else {
                    flag.set(true);
                    Some(ReentryGuard)
                }
            })
            .ok()
            .flatten()
    }
}

impl Drop for ReentryGuard {
    fn drop(&mut self) {
        let _ = IN_ALLOC.try_with(|flag| flag.set(false));
    }
}

/// Global allocator wrapper that tracks allocations.
///
/// Install as `#[global_allocator]` to enable tracking. The wrapper
/// forwards every allocation, deallocation, reallocation, and
/// zero-initialised allocation to [`std::alloc::System`] and records
/// the event in four lock-free [`AtomicU64`] counters.
///
/// # Example
///
/// ```no_run
/// use mod_alloc::ModAlloc;
///
/// #[global_allocator]
/// static GLOBAL: ModAlloc = ModAlloc::new();
///
/// fn main() {
///     let v: Vec<u8> = vec![0; 1024];
///     let stats = GLOBAL.snapshot();
///     assert!(stats.alloc_count >= 1);
///     drop(v);
/// }
/// ```
pub struct ModAlloc {
    alloc_count: AtomicU64,
    total_bytes: AtomicU64,
    peak_bytes: AtomicU64,
    current_bytes: AtomicU64,
}

impl ModAlloc {
    /// Construct a new `ModAlloc` allocator wrapper.
    ///
    /// All counters start at zero. This function is `const`, which
    /// allows construction in a `static` for use as
    /// `#[global_allocator]`.
    ///
    /// # Example
    ///
    /// ```
    /// use mod_alloc::ModAlloc;
    ///
    /// static GLOBAL: ModAlloc = ModAlloc::new();
    /// let stats = GLOBAL.snapshot();
    /// assert_eq!(stats.alloc_count, 0);
    /// ```
    pub const fn new() -> Self {
        Self {
            alloc_count: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
            peak_bytes: AtomicU64::new(0),
            current_bytes: AtomicU64::new(0),
        }
    }

    /// Snapshot the current counter values.
    ///
    /// Each counter is read independently with `Relaxed` ordering;
    /// the resulting [`AllocStats`] is a coherent best-effort view
    /// but does not represent a single atomic moment in time. For
    /// scoped measurement, prefer [`Profiler`].
    ///
    /// # Example
    ///
    /// ```
    /// use mod_alloc::ModAlloc;
    ///
    /// let alloc = ModAlloc::new();
    /// let stats = alloc.snapshot();
    /// assert_eq!(stats.alloc_count, 0);
    /// ```
    pub fn snapshot(&self) -> AllocStats {
        AllocStats {
            alloc_count: self.alloc_count.load(Ordering::Relaxed),
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            peak_bytes: self.peak_bytes.load(Ordering::Relaxed),
            current_bytes: self.current_bytes.load(Ordering::Relaxed),
        }
    }

    /// Reset all counters to zero.
    ///
    /// Intended for use at the start of a profile run, before any
    /// outstanding allocations exist. Calling `reset` while
    /// allocations are live can cause `current_bytes` to wrap on
    /// subsequent deallocations; the other counters are unaffected.
    ///
    /// # Example
    ///
    /// ```
    /// use mod_alloc::ModAlloc;
    ///
    /// let alloc = ModAlloc::new();
    /// alloc.reset();
    /// let stats = alloc.snapshot();
    /// assert_eq!(stats.alloc_count, 0);
    /// ```
    pub fn reset(&self) {
        self.alloc_count.store(0, Ordering::Relaxed);
        self.total_bytes.store(0, Ordering::Relaxed);
        self.peak_bytes.store(0, Ordering::Relaxed);
        self.current_bytes.store(0, Ordering::Relaxed);
    }

    #[inline]
    fn record_alloc(&self, size: u64) {
        self.alloc_count.fetch_add(1, Ordering::Relaxed);
        self.total_bytes.fetch_add(size, Ordering::Relaxed);
        let new_current = self.current_bytes.fetch_add(size, Ordering::Relaxed) + size;
        self.peak_bytes.fetch_max(new_current, Ordering::Relaxed);
    }

    #[inline]
    fn record_dealloc(&self, size: u64) {
        self.current_bytes.fetch_sub(size, Ordering::Relaxed);
    }

    #[inline]
    fn record_realloc(&self, old_size: u64, new_size: u64) {
        self.alloc_count.fetch_add(1, Ordering::Relaxed);
        if new_size > old_size {
            let delta = new_size - old_size;
            self.total_bytes.fetch_add(delta, Ordering::Relaxed);
            let new_current = self.current_bytes.fetch_add(delta, Ordering::Relaxed) + delta;
            self.peak_bytes.fetch_max(new_current, Ordering::Relaxed);
        } else if new_size < old_size {
            self.current_bytes
                .fetch_sub(old_size - new_size, Ordering::Relaxed);
        }
    }

    #[inline]
    fn register_self(&self) {
        if GLOBAL_HANDLE.load(Ordering::Relaxed).is_null() {
            let _ = GLOBAL_HANDLE.compare_exchange(
                ptr::null_mut(),
                self as *const ModAlloc as *mut ModAlloc,
                Ordering::Release,
                Ordering::Relaxed,
            );
        }
    }
}

impl Default for ModAlloc {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: `ModAlloc` adds counter bookkeeping but performs all
// underlying allocation through [`std::alloc::System`]. Each method
// forwards its arguments unchanged to `System` and only inspects
// the result; size/alignment invariants required by the
// `GlobalAlloc` contract are passed through unmodified, so the
// caller's contract to us becomes our contract to System.
//
// The counter-update path uses thread-local reentrancy detection
// (see `ReentryGuard`) so that any allocation triggered transitively
// inside the tracking path bypasses tracking and forwards directly
// to System, preserving the "hook MUST NOT itself allocate"
// invariant from REPS section 4.
unsafe impl GlobalAlloc for ModAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: per `GlobalAlloc::alloc`, `layout` has non-zero
        // size; we forward unchanged to `System.alloc`, which has
        // the same contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            if let Some(_g) = ReentryGuard::enter() {
                self.record_alloc(layout.size() as u64);
                self.register_self();
            }
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: same invariants as `alloc`; `layout` forwarded
        // unchanged. `System.alloc_zeroed` zero-fills the returned
        // memory, satisfying the `GlobalAlloc::alloc_zeroed`
        // contract.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            if let Some(_g) = ReentryGuard::enter() {
                self.record_alloc(layout.size() as u64);
                self.register_self();
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: per `GlobalAlloc::dealloc`, `ptr` was returned by a
        // prior call to `alloc`/`alloc_zeroed`/`realloc` on this
        // allocator with the given `layout`; we forwarded all of
        // those to `System` with the same `layout`, so the inverse
        // pairing for `System.dealloc(ptr, layout)` is valid.
        unsafe { System.dealloc(ptr, layout) };
        if let Some(_g) = ReentryGuard::enter() {
            self.record_dealloc(layout.size() as u64);
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: per `GlobalAlloc::realloc`, `ptr` was returned by
        // a prior allocation with `layout`, `new_size` is non-zero,
        // and the alignment in `layout` remains valid for the new
        // size. We forward all three to `System.realloc` which has
        // the same contract.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if let Some(_g) = ReentryGuard::enter() {
                self.record_realloc(layout.size() as u64, new_size as u64);
                self.register_self();
            }
        }
        new_ptr
    }
}

/// Snapshot of allocation statistics at a point in time.
///
/// Produced by [`ModAlloc::snapshot`] and [`Profiler::stop`].
///
/// # Example
///
/// ```
/// use mod_alloc::AllocStats;
///
/// let stats = AllocStats {
///     alloc_count: 10,
///     total_bytes: 1024,
///     peak_bytes: 512,
///     current_bytes: 256,
/// };
/// assert_eq!(stats.alloc_count, 10);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocStats {
    /// Number of allocations performed.
    pub alloc_count: u64,
    /// Total bytes allocated across all allocations. Reallocations
    /// contribute the growth delta (or zero on shrink).
    pub total_bytes: u64,
    /// Peak resident bytes (highest `current_bytes` ever observed).
    pub peak_bytes: u64,
    /// Currently-allocated bytes (allocations minus deallocations).
    pub current_bytes: u64,
}

/// Scoped profiler that captures a delta between start and stop.
///
/// Read the snapshot of the installed [`ModAlloc`] on construction
/// and again on [`Profiler::stop`], returning the difference. If no
/// `ModAlloc` is installed as `#[global_allocator]` and no
/// allocation has occurred through it yet, both snapshots are
/// zero and the delta is zero.
///
/// # Example
///
/// ```no_run
/// use mod_alloc::{ModAlloc, Profiler};
///
/// #[global_allocator]
/// static GLOBAL: ModAlloc = ModAlloc::new();
///
/// fn main() {
///     let p = Profiler::start();
///     let v: Vec<u8> = vec![0; 1024];
///     drop(v);
///     let stats = p.stop();
///     println!("Captured {} alloc events", stats.alloc_count);
/// }
/// ```
pub struct Profiler {
    baseline: AllocStats,
}

impl Profiler {
    /// Begin profiling, capturing the current allocation state.
    ///
    /// If no `ModAlloc` is installed as `#[global_allocator]` or no
    /// allocation has occurred yet, the captured baseline is all
    /// zeros.
    ///
    /// # Example
    ///
    /// ```
    /// use mod_alloc::Profiler;
    ///
    /// let p = Profiler::start();
    /// let _delta = p.stop();
    /// ```
    pub fn start() -> Self {
        Self {
            baseline: current_snapshot_or_zeros(),
        }
    }

    /// Stop profiling and return the delta from start.
    ///
    /// `alloc_count`, `total_bytes`, and `current_bytes` are deltas
    /// from `start()` to `stop()`. `peak_bytes` is the absolute
    /// high-water mark observed during the profiling window (peak
    /// has no meaningful delta semantic).
    ///
    /// # Example
    ///
    /// ```
    /// use mod_alloc::Profiler;
    ///
    /// let p = Profiler::start();
    /// let stats = p.stop();
    /// assert_eq!(stats.alloc_count, 0);
    /// ```
    pub fn stop(self) -> AllocStats {
        let now = current_snapshot_or_zeros();
        AllocStats {
            alloc_count: now.alloc_count.saturating_sub(self.baseline.alloc_count),
            total_bytes: now.total_bytes.saturating_sub(self.baseline.total_bytes),
            current_bytes: now
                .current_bytes
                .saturating_sub(self.baseline.current_bytes),
            peak_bytes: now.peak_bytes,
        }
    }
}

fn current_snapshot_or_zeros() -> AllocStats {
    let p = GLOBAL_HANDLE.load(Ordering::Acquire);
    if p.is_null() {
        AllocStats {
            alloc_count: 0,
            total_bytes: 0,
            peak_bytes: 0,
            current_bytes: 0,
        }
    } else {
        // SAFETY: `GLOBAL_HANDLE` is only ever set by
        // `ModAlloc::register_self` to point at the address of a
        // `#[global_allocator] static` (or any other `'static`
        // `ModAlloc`). That target has `'static` lifetime, so the
        // pointer remains valid for the remainder of the program.
        // We produce only a shared borrow used to call `&self`
        // methods that read atomic counters; no mutation through
        // the pointer occurs here.
        unsafe { (*p).snapshot() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocator_constructs() {
        let _ = ModAlloc::new();
    }

    #[test]
    fn snapshot_returns_zeros_initially() {
        let a = ModAlloc::new();
        let s = a.snapshot();
        assert_eq!(s.alloc_count, 0);
        assert_eq!(s.total_bytes, 0);
        assert_eq!(s.peak_bytes, 0);
        assert_eq!(s.current_bytes, 0);
    }

    #[test]
    fn reset_works() {
        let a = ModAlloc::new();
        a.reset();
        let s = a.snapshot();
        assert_eq!(s.alloc_count, 0);
    }

    #[test]
    fn record_alloc_updates_counters() {
        let a = ModAlloc::new();
        a.record_alloc(128);
        a.record_alloc(256);
        let s = a.snapshot();
        assert_eq!(s.alloc_count, 2);
        assert_eq!(s.total_bytes, 384);
        assert_eq!(s.current_bytes, 384);
        assert_eq!(s.peak_bytes, 384);
    }

    #[test]
    fn record_dealloc_decreases_current_only() {
        let a = ModAlloc::new();
        a.record_alloc(1000);
        a.record_dealloc(400);
        let s = a.snapshot();
        assert_eq!(s.alloc_count, 1);
        assert_eq!(s.total_bytes, 1000);
        assert_eq!(s.current_bytes, 600);
        assert_eq!(s.peak_bytes, 1000);
    }

    #[test]
    fn record_realloc_growth_updates_total_and_peak() {
        let a = ModAlloc::new();
        a.record_alloc(100);
        a.record_realloc(100, 250);
        let s = a.snapshot();
        assert_eq!(s.alloc_count, 2);
        assert_eq!(s.total_bytes, 250);
        assert_eq!(s.current_bytes, 250);
        assert_eq!(s.peak_bytes, 250);
    }

    #[test]
    fn record_realloc_shrink_only_adjusts_current() {
        let a = ModAlloc::new();
        a.record_alloc(500);
        a.record_realloc(500, 200);
        let s = a.snapshot();
        assert_eq!(s.alloc_count, 2);
        assert_eq!(s.total_bytes, 500);
        assert_eq!(s.current_bytes, 200);
        assert_eq!(s.peak_bytes, 500);
    }

    #[test]
    fn peak_holds_high_water_mark() {
        let a = ModAlloc::new();
        a.record_alloc(1000);
        a.record_dealloc(1000);
        a.record_alloc(500);
        let s = a.snapshot();
        assert_eq!(s.peak_bytes, 1000);
        assert_eq!(s.current_bytes, 500);
    }

    #[test]
    fn reentry_guard_blocks_nested_entry() {
        let outer = ReentryGuard::enter();
        assert!(outer.is_some());
        let inner = ReentryGuard::enter();
        assert!(inner.is_none(), "nested entry must be denied");
        drop(outer);
        let after = ReentryGuard::enter();
        assert!(after.is_some(), "entry must be allowed after outer drops");
    }

    #[test]
    fn profiler_start_stop_with_no_handle() {
        let p = Profiler::start();
        let s = p.stop();
        assert_eq!(s.alloc_count, 0);
    }
}
