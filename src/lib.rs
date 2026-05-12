//! # mod-alloc
//!
//! Allocation profiling for Rust. Tracks allocation counts, total
//! bytes, and peak resident memory. Optionally captures call-sites
//! via inline backtrace (no `backtrace` crate dependency).
//!
//! Designed as a leaner replacement for `dhat` with MSRV 1.75 and
//! zero external dependencies on the hot path.
//!
//! ## Quick example
//!
//! ```ignore
//! use mod_alloc::{Profiler, ModAlloc};
//!
//! #[global_allocator]
//! static GLOBAL: ModAlloc = ModAlloc::new();
//!
//! fn main() {
//!     let p = Profiler::start();
//!     // ... do work ...
//!     let stats = p.stop();
//!     println!("Allocs: {}, Peak: {} bytes", stats.alloc_count, stats.peak_bytes);
//! }
//! ```
//!
//! Note: the `#[global_allocator]` integration is not yet wired in
//! `v0.1.0`. Real `GlobalAlloc` implementation lands in `0.9.x`.
//! For now, `ModAlloc` exposes the API surface and works as a
//! standalone counter object.
//!
//! ## Working API in v0.1.0
//!
//! ```
//! use mod_alloc::{ModAlloc, Profiler};
//!
//! let alloc = ModAlloc::new();
//! let stats = alloc.snapshot();
//! assert_eq!(stats.alloc_count, 0);
//!
//! let p = Profiler::start();
//! let delta = p.stop();
//! assert_eq!(delta.alloc_count, 0);
//! ```
//!
//! ## Status
//!
//! `v0.1.0` is the name-claim release. Real `GlobalAlloc`
//! implementation lands in `0.9.x`. The placeholder here lets the
//! crate compile and reserves the API surface.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

use std::sync::atomic::{AtomicU64, Ordering};

/// Global allocator wrapper that tracks allocations.
///
/// Install as `#[global_allocator]` to enable tracking.
///
/// In `0.1.0` this is a placeholder forwarding to `System`. The real
/// per-thread arena-based implementation lands in `0.9.x`.
pub struct ModAlloc {
    alloc_count: AtomicU64,
    total_bytes: AtomicU64,
    peak_bytes: AtomicU64,
    current_bytes: AtomicU64,
}

impl ModAlloc {
    /// Construct a new `ModAlloc` allocator wrapper.
    pub const fn new() -> Self {
        Self {
            alloc_count: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
            peak_bytes: AtomicU64::new(0),
            current_bytes: AtomicU64::new(0),
        }
    }

    /// Snapshot the current counters.
    pub fn snapshot(&self) -> AllocStats {
        AllocStats {
            alloc_count: self.alloc_count.load(Ordering::Relaxed),
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            peak_bytes: self.peak_bytes.load(Ordering::Relaxed),
            current_bytes: self.current_bytes.load(Ordering::Relaxed),
        }
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.alloc_count.store(0, Ordering::Relaxed);
        self.total_bytes.store(0, Ordering::Relaxed);
        self.peak_bytes.store(0, Ordering::Relaxed);
        self.current_bytes.store(0, Ordering::Relaxed);
    }
}

impl Default for ModAlloc {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of allocation statistics at a point in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocStats {
    /// Number of allocations performed.
    pub alloc_count: u64,
    /// Total bytes allocated across all allocations.
    pub total_bytes: u64,
    /// Peak resident bytes (highest `current_bytes` ever observed).
    pub peak_bytes: u64,
    /// Currently-allocated bytes (allocations minus deallocations).
    pub current_bytes: u64,
}

/// Scoped profiler that captures a delta between start and stop.
///
/// In `0.1.0` this is a placeholder. Real per-thread arena tracking
/// lands in `0.9.x`.
pub struct Profiler {
    start: AllocStats,
}

impl Profiler {
    /// Begin profiling, capturing the current allocation state.
    pub fn start() -> Self {
        Self {
            start: AllocStats {
                alloc_count: 0,
                total_bytes: 0,
                peak_bytes: 0,
                current_bytes: 0,
            },
        }
    }

    /// Stop profiling and return the delta from start.
    pub fn stop(self) -> AllocStats {
        // Placeholder: returns the captured start state. Real
        // implementation lands in 0.9.x.
        self.start
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
    fn profiler_start_stop() {
        let p = Profiler::start();
        let _ = p.stop();
    }
}
