//! dhat-rs-shaped statistics types.
//!
//! Mirrors the public surface of `dhat::HeapStats` and
//! `dhat::AdHocStats` so downstream consumers can swap dhat for
//! mod-alloc with no field renames.

use std::sync::atomic::{AtomicU64, Ordering};

/// Heap-mode statistics snapshot. Mirrors the shape of
/// `dhat::HeapStats` field-for-field so consumers migrating from
/// dhat can drop in the type alias without code edits.
///
/// Note the `total_*` fields are `u64` while `curr_*` and `max_*`
/// are `usize` — this matches dhat-rs exactly. On 32-bit targets
/// the `u64 -> usize` casts in [`HeapStats::get`] are saturating;
/// in practice mod-alloc targets 64-bit only (the Tier 2 walker
/// requires `x86_64` / `aarch64`).
///
/// # Example
///
/// ```no_run
/// # #[cfg(feature = "dhat-compat")]
/// # fn demo() {
/// use mod_alloc::dhat_compat::{Alloc, HeapStats};
///
/// #[global_allocator]
/// static ALLOC: Alloc = Alloc;
///
/// let _v: Vec<u8> = vec![0; 1024];
/// let stats = HeapStats::get();
/// assert!(stats.total_bytes >= 1024);
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeapStats {
    /// Total blocks ever allocated (lifetime count).
    pub total_blocks: u64,
    /// Total bytes ever allocated (lifetime sum).
    pub total_bytes: u64,
    /// Currently-alive block count.
    pub curr_blocks: usize,
    /// Currently-resident bytes.
    pub curr_bytes: usize,
    /// Peak live block count.
    pub max_blocks: usize,
    /// Peak resident bytes.
    pub max_bytes: usize,
}

impl HeapStats {
    /// Snapshot the current heap statistics from the installed
    /// allocator.
    ///
    /// While a [`crate::dhat_compat::Profiler`] is alive, the figures
    /// cover only the profiling session, as in dhat-rs: `total_*`
    /// count allocations since the first live Profiler was built,
    /// `curr_*` are the change in live blocks and bytes since then,
    /// and `max_*` are the peaks reached during the session above
    /// what was live when it began. With no Profiler alive, the
    /// figures cover the whole process. (Before 1.1.0 they always
    /// covered the whole process.)
    ///
    /// If neither [`crate::dhat_compat::Alloc`] nor
    /// [`crate::ModAlloc`] is the `#[global_allocator]`, all fields
    /// are zero.
    pub fn get() -> Self {
        let session = super::profiler::lock_session();
        let Some(handle) = crate::installed_handle() else {
            return Self::from_stats(&crate::zero_stats());
        };
        let snap = handle.snapshot();
        if !session.is_open() {
            return Self::from_stats(&snap);
        }
        let base = session.baseline;
        let (peak_bytes, peak_live) = match session.window {
            Some(slot) => handle.window_peaks(slot),
            None => (snap.peak_bytes, snap.peak_live_count),
        };
        drop(session);
        let curr_bytes = snap.current_bytes.saturating_sub(base.current_bytes);
        let curr_blocks = snap.live_count.saturating_sub(base.live_count);
        Self {
            total_blocks: snap.alloc_count.saturating_sub(base.alloc_count),
            total_bytes: snap.total_bytes.saturating_sub(base.total_bytes),
            curr_blocks: to_usize(curr_blocks),
            curr_bytes: to_usize(curr_bytes),
            max_blocks: to_usize(peak_live.saturating_sub(base.live_count).max(curr_blocks)),
            max_bytes: to_usize(
                peak_bytes
                    .saturating_sub(base.current_bytes)
                    .max(curr_bytes),
            ),
        }
    }

    fn from_stats(snap: &crate::AllocStats) -> Self {
        Self {
            total_blocks: snap.alloc_count,
            total_bytes: snap.total_bytes,
            curr_blocks: to_usize(snap.live_count),
            curr_bytes: to_usize(snap.current_bytes),
            max_blocks: to_usize(snap.peak_live_count),
            max_bytes: to_usize(snap.peak_bytes),
        }
    }
}

fn to_usize(v: u64) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// Ad-hoc-mode statistics snapshot. Mirrors `dhat::AdHocStats`.
///
/// Populated by [`ad_hoc_event`] calls, which are independent of
/// the heap allocator hot path.
///
/// # Example
///
/// ```no_run
/// # #[cfg(feature = "dhat-compat")]
/// # fn demo() {
/// use mod_alloc::dhat_compat::{ad_hoc_event, AdHocStats};
///
/// ad_hoc_event(42);
/// let stats = AdHocStats::get();
/// assert_eq!(stats.total_events, 1);
/// assert_eq!(stats.total_units, 42);
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AdHocStats {
    /// Number of [`ad_hoc_event`] calls.
    pub total_events: u64,
    /// Sum of all `weight` arguments passed to [`ad_hoc_event`].
    pub total_units: u64,
}

impl AdHocStats {
    /// Snapshot the current ad-hoc statistics.
    pub fn get() -> Self {
        Self {
            total_events: AD_HOC_EVENTS.load(Ordering::Relaxed),
            total_units: AD_HOC_UNITS.load(Ordering::Relaxed),
        }
    }
}

pub(crate) static AD_HOC_EVENTS: AtomicU64 = AtomicU64::new(0);
pub(crate) static AD_HOC_UNITS: AtomicU64 = AtomicU64::new(0);

/// Record one ad-hoc event with the given weight.
///
/// Two atomic operations per call; no allocation. Safe to call
/// from any context, including transitively from inside the
/// allocator hook.
///
/// # Example
///
/// ```no_run
/// # #[cfg(feature = "dhat-compat")]
/// # {
/// use mod_alloc::dhat_compat::ad_hoc_event;
///
/// ad_hoc_event(1);     // one unit of work
/// ad_hoc_event(1024);  // a chunk of work
/// # }
/// ```
pub fn ad_hoc_event(weight: usize) {
    AD_HOC_EVENTS.fetch_add(1, Ordering::Relaxed);
    AD_HOC_UNITS.fetch_add(weight as u64, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ad_hoc_event_accumulates() {
        let before = AdHocStats::get();
        ad_hoc_event(10);
        ad_hoc_event(5);
        let after = AdHocStats::get();
        assert_eq!(after.total_events - before.total_events, 2);
        assert_eq!(after.total_units - before.total_units, 15);
    }
}
