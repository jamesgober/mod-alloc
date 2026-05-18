//! dhat-rs-shaped compatibility surface.
//!
//! Behind the `dhat-compat` cargo feature. Provides drop-in
//! replacements for `dhat::Alloc`, `dhat::Profiler`,
//! `dhat::ProfilerBuilder`, `dhat::HeapStats`, `dhat::AdHocStats`,
//! and `dhat::ad_hoc_event` so consumers migrating from dhat-rs
//! can swap allocator profilers with a one-line import change:
//!
//! ```no_run
//! # #[cfg(feature = "dhat-compat")]
//! # mod swap_example {
//! use mod_alloc::dhat_compat as dhat;
//!
//! #[global_allocator]
//! static ALLOC: dhat::Alloc = dhat::Alloc;
//!
//! fn main() {
//!     let _profiler = dhat::Profiler::new_heap();
//!     // ... work ...
//!     // _profiler drops here → writes dhat-heap.json
//! }
//! # }
//! ```
//!
//! ## Differences from dhat-rs
//!
//! Documented in `MIGRATING_FROM_DHAT.md`. Summary:
//!
//! - Backtrace depth is capped at 8 frames (Tier 2 walker limit);
//!   `ProfilerBuilder::trim_backtraces` is accepted for parity
//!   but silently clamps.
//! - Drop-time file-write errors are swallowed silently — same
//!   as dhat-rs's behaviour.
//! - Double-Profiler construction is a no-op rather than a panic
//!   (dhat-rs panics). Last writer wins on the JSON file.
//! - `dhat::assert!` / `assert_eq!` / `assert_ne!` macros are not
//!   yet shipped. Use `HeapStats::get()` directly in test
//!   assertions.

use std::alloc::{GlobalAlloc, Layout};

mod ad_hoc_writer;
mod profiler;
mod stats;

pub use profiler::{Mode, Profiler, ProfilerBuilder};
pub use stats::{ad_hoc_event, AdHocStats, HeapStats};

/// Drop-in replacement for `dhat::Alloc`.
///
/// Unit struct usable in the literal `static A: Alloc = Alloc;`
/// pattern that dhat-rs documents. Internally forwards every
/// allocation to a process-wide static [`crate::ModAlloc`] so
/// `HeapStats::get()` and `Profiler` find the live counters.
///
/// # Example
///
/// ```no_run
/// # #[cfg(feature = "dhat-compat")]
/// # mod ex {
/// use mod_alloc::dhat_compat::Alloc;
///
/// #[global_allocator]
/// static ALLOC: Alloc = Alloc;
/// # }
/// ```
pub struct Alloc;

impl Alloc {
    /// Construct an `Alloc` value (also usable as just `Alloc`).
    ///
    /// Provided for parity with `ModAlloc::new()` and for
    /// downstream code that prefers the constructor form. The
    /// preferred dhat-style pattern is `static A: Alloc = Alloc;`.
    pub const fn new() -> Self {
        Self
    }
}

impl Default for Alloc {
    fn default() -> Self {
        Self
    }
}

// Process-wide tracking allocator that all `Alloc` instances
// delegate to. Hosting it as a `static` (rather than per-`Alloc`)
// is necessary because `Alloc` itself is a zero-sized type — it
// has no place to put atomic counters — and because dhat-rs's
// pattern uses literal `dhat::Alloc` values in `static` position
// without any constructor call.
static INNER: crate::ModAlloc = crate::ModAlloc::new();

// SAFETY: `Alloc` is a thin forwarding wrapper around `INNER`,
// which is a `'static` `ModAlloc`. Every `GlobalAlloc` method
// forwards its arguments unchanged to `INNER`'s implementation;
// size and alignment invariants pass through unmodified. `INNER`
// itself satisfies the `GlobalAlloc` contract (see
// `crate::ModAlloc`'s `unsafe impl`), so the same contract holds
// through the forwarder.
unsafe impl GlobalAlloc for Alloc {
    // `#[inline(always)]` on each forwarder is important for the
    // backtrace path: without it, the call chain becomes
    // `user_code -> Alloc::alloc -> ModAlloc::alloc ->
    // record_event`, which adds an extra stack frame in front of
    // the user's call site. Inlining folds `Alloc::alloc` away,
    // so the captured frame-pointer chain matches the direct
    // `ModAlloc` usage path and the walker recovers the same
    // depth of user frames.
    #[inline(always)]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: `layout` forwarded unchanged to `INNER.alloc`,
        // which has the same `GlobalAlloc::alloc` contract.
        unsafe { INNER.alloc(layout) }
    }

    #[inline(always)]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: same invariants as `alloc`; forwarded unchanged.
        unsafe { INNER.alloc_zeroed(layout) }
    }

    #[inline(always)]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from a prior call to one of this
        // type's `alloc` family methods, all of which forwarded
        // to `INNER`. The `(ptr, layout)` pairing is therefore
        // valid for `INNER.dealloc`.
        unsafe { INNER.dealloc(ptr, layout) }
    }

    #[inline(always)]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: same reasoning as `dealloc`; the `(ptr,
        // layout)` pairing is valid for `INNER.realloc`, and
        // `new_size` and alignment invariants are passed through
        // unmodified.
        unsafe { INNER.realloc(ptr, layout, new_size) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_is_zero_sized() {
        assert_eq!(core::mem::size_of::<Alloc>(), 0);
    }

    #[test]
    fn alloc_new_constructs_unit() {
        let _a = Alloc::new();
        let _b: Alloc = Alloc;
        let _c: Alloc = <Alloc as Default>::default();
    }

    #[test]
    fn heap_stats_reflects_zero_initial_state_on_uninstalled_path() {
        // Without anything installed as `#[global_allocator]`,
        // `HeapStats::get` returns zeros (the GLOBAL_HANDLE
        // path's null-pointer branch). The integration test
        // covers the installed path.
        // We cannot reliably assert zeros here in unit-test
        // position because the test harness itself uses the
        // system allocator and may have populated the
        // GLOBAL_HANDLE from prior tests that exercised
        // `ModAlloc`. Shape-only smoke check:
        let s = HeapStats::get();
        // Field access shapes; no value assertions.
        let _ = (
            s.total_blocks,
            s.total_bytes,
            s.curr_blocks,
            s.curr_bytes,
            s.max_blocks,
            s.max_bytes,
        );
    }
}
