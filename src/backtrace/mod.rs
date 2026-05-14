//! Tier 2 backtrace capture for `mod-alloc`.
//!
//! Behind the `backtraces` cargo feature. Each tracked allocation
//! captures up to 8 frames of its call site via inline frame-
//! pointer walking on `x86_64` and `aarch64`, aggregates into a
//! process-wide table keyed by frame hash, and exposes the result
//! via [`ModAlloc::call_sites`](crate::ModAlloc::call_sites).
//!
//! Symbolication (resolving frame addresses to function names)
//! lands in `v0.9.2`. DHAT-format JSON output lands in `v0.9.3`.
//!
//! ## Requirements
//!
//! The user must compile with frame pointers enabled. `build.rs`
//! emits a warning at build time if `RUSTFLAGS` is missing
//! `-C force-frame-pointers=yes`. Without frame pointers, traces
//! return zero or one frame and the per-call-site report has a
//! single uninformative bucket.
//!
//! ## Configuration
//!
//! Aggregation-table size is controlled by the `MOD_ALLOC_BUCKETS`
//! environment variable at process start. Default is 4,096
//! buckets (~384 KB). The value is rounded up to the next power
//! of two and clamped to `[64, 1,048,576]`.

pub(crate) mod arena;
pub(crate) mod capture;
pub(crate) mod hash;
pub(crate) mod raw_mem;
pub(crate) mod stack_bounds;
pub(crate) mod table;
pub(crate) mod walk;

pub use table::{call_sites_report, CallSiteStats};

use capture::current_fp;
use stack_bounds::current_stack_bounds;
use walk::{walk, Frames};

/// Capture and record one event at the call site of this function.
///
/// Called from the `GlobalAlloc` impl after the Tier 1 counter
/// update on every tracked alloc / alloc_zeroed / realloc event.
/// `size` is the bytes attributed to this event (per-event policy
/// matches `dhat`: alloc reports its size, realloc reports
/// new_size including shrinks).
///
/// Marked `#[inline(always)]` so the captured FP corresponds to
/// the calling `alloc` method, not this helper.
#[inline(always)]
pub(crate) fn record_event(size: u64) {
    let fp = current_fp();
    if fp == 0 {
        return;
    }
    let bounds = match current_stack_bounds() {
        Some(b) => b,
        None => return,
    };
    let frames: Frames = walk(fp, bounds);
    if frames.count == 0 {
        return;
    }
    arena::record_event(&frames, size);
}
