//! Global call-site aggregation table.
//!
//! Open-addressed, fixed-size, atomic-only hash table allocated
//! once via raw OS pages. `record_event` writes directly here on
//! every tracked alloc; the public report API drains it into a
//! `Vec<CallSiteStats>`. The per-thread arena that v0.9.1–v0.9.5
//! used as a batching layer was removed in v0.9.6 because the
//! table's matching-path fast write (two atomic ops) is already
//! cheap enough that batching added more cost than it saved.
//!
//! ## Sizing
//!
//! Default: 4,096 buckets × 96 bytes = ~384 KB. Override via the
//! `MOD_ALLOC_BUCKETS` environment variable at process start. The
//! value is rounded up to the next power of two and clamped to
//! `[64, 1,048,576]`. Reading the env var allocates a small
//! `String` on first call; this happens inside the allocator hook
//! with the reentrancy guard set, so the recursive `alloc` is
//! forwarded directly to `System` without tracking.
//!
//! ## Concurrency
//!
//! Buckets use a two-phase publish protocol: the hash field is
//! CAS-claimed first, then `sample_frames` are written, then
//! `frame_count` is stored with `Release` to mark the bucket
//! fully populated. Readers gate on `frame_count > 0` after
//! observing a non-zero hash; this avoids torn reads of the
//! sample frames.

use std::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering};

use super::hash::hash_frames;
use super::raw_mem::{alloc_pages, free_pages};
use super::walk::Frames;

const DEFAULT_BUCKETS: usize = 4_096;
const MIN_BUCKETS: usize = 64;
const MAX_BUCKETS: usize = 1 << 20; // 1,048,576

/// One row in the per-call-site report.
///
/// Frames are raw return addresses, top of stack first. The
/// `frames[..frame_count]` slice is the captured trace; the rest
/// is zero. Symbolication happens at report-generation time
/// (shipped in v0.9.2 behind the `symbolicate` feature).
///
/// # Stability
///
/// Marked `#[non_exhaustive]` as of v1.0.0. New counter fields
/// (e.g. per-bucket high-water marks) may be added in future
/// minor versions without bumping the major version. Reading
/// fields by name is fully stable. Iterate via
/// [`ModAlloc::call_sites`](crate::ModAlloc::call_sites) rather
/// than constructing literals.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct CallSiteStats {
    /// Raw return addresses, top of stack first.
    pub frames: [u64; 8],
    /// Number of valid frames in `frames`.
    pub frame_count: u8,
    /// Number of allocations attributed to this call site.
    pub count: u64,
    /// Total bytes allocated at this call site (across all
    /// recorded events).
    pub total_bytes: u64,
}

#[repr(C, align(16))]
struct Bucket {
    hash: AtomicU64,
    count: AtomicU64,
    total_bytes: AtomicU64,
    frame_count: AtomicU64,
    sample_frames: [AtomicU64; 8],
}

const BUCKET_SIZE: usize = core::mem::size_of::<Bucket>();

static TABLE_BASE: AtomicPtr<Bucket> = AtomicPtr::new(core::ptr::null_mut());
static TABLE_BUCKETS: AtomicUsize = AtomicUsize::new(0);
static TABLE_MASK: AtomicUsize = AtomicUsize::new(0);

fn configured_bucket_count() -> usize {
    let raw = match std::env::var("MOD_ALLOC_BUCKETS") {
        Ok(s) => s,
        Err(_) => return DEFAULT_BUCKETS,
    };
    let n: usize = raw.trim().parse().unwrap_or(DEFAULT_BUCKETS);
    let n = n.clamp(MIN_BUCKETS, MAX_BUCKETS);
    n.next_power_of_two()
}

/// Steady-state inline accessor. Folds into the calling
/// `table::record` body so the fast path (table already
/// initialised, which is true after the very first event for
/// the process) becomes three atomic loads with no function-
/// call boundary.
#[inline(always)]
fn ensure_init() -> Option<(*mut Bucket, usize, usize)> {
    let existing = TABLE_BASE.load(Ordering::Acquire);
    if !existing.is_null() {
        let buckets = TABLE_BUCKETS.load(Ordering::Acquire);
        let mask = TABLE_MASK.load(Ordering::Relaxed);
        return Some((existing, buckets, mask));
    }
    ensure_init_slow()
}

/// First-event-per-process slow path. Allocates the table via
/// raw OS pages and CAS-publishes it. Kept out-of-line so the
/// `record` hot path stays compact.
#[cold]
#[inline(never)]
fn ensure_init_slow() -> Option<(*mut Bucket, usize, usize)> {
    let buckets = configured_bucket_count();
    let bytes = buckets * BUCKET_SIZE;
    // SAFETY: alloc_pages returns either null or a writable,
    // zero-init region of `bytes` bytes.
    let pages = unsafe { alloc_pages(bytes) };
    if pages.is_null() {
        return None;
    }
    let new_base = pages as *mut Bucket;

    match TABLE_BASE.compare_exchange(
        core::ptr::null_mut(),
        new_base,
        Ordering::Release,
        Ordering::Acquire,
    ) {
        Ok(_) => {
            TABLE_BUCKETS.store(buckets, Ordering::Release);
            TABLE_MASK.store(buckets - 1, Ordering::Release);
            Some((new_base, buckets, buckets - 1))
        }
        Err(other) => {
            // SAFETY: we own `pages`; the CAS loser has not used it.
            unsafe { free_pages(pages, bytes) };
            // Wait until the winner publishes the count.
            loop {
                let b = TABLE_BUCKETS.load(Ordering::Acquire);
                if b > 0 {
                    let mask = TABLE_MASK.load(Ordering::Relaxed);
                    return Some((other, b, mask));
                }
                core::hint::spin_loop();
            }
        }
    }
}

/// Record one captured event into the global table.
///
/// Called from `backtrace::record_event` on every tracked alloc.
/// Marked `#[inline]` so thin-LTO can stitch the hot path
/// (record_event → hash_frames → record) into the calling
/// `GlobalAlloc::alloc` body without a cross-module function
/// call.
#[inline]
pub(crate) fn record(frames: &Frames, size: u64) {
    let count = frames.count as usize;
    if count == 0 {
        // Nothing meaningful to bucket on a zero-frame capture
        // (target unsupported, FP unavailable, etc.). Skip.
        return;
    }
    let Some((base, _buckets, mask)) = ensure_init() else {
        return;
    };
    let h = hash_frames(&frames.frames, count);
    let mut idx = (h as usize) & mask;
    let start = idx;

    loop {
        // SAFETY: `base` is the start of an array of length
        // `mask + 1`, and `idx <= mask`. Pointer arithmetic stays
        // in-bounds.
        let bucket = unsafe { &*base.add(idx) };
        let existing = bucket.hash.load(Ordering::Acquire);

        if existing == 0 {
            match bucket
                .hash
                .compare_exchange(0, h, Ordering::Release, Ordering::Acquire)
            {
                Ok(_) => {
                    // We own the initialisation phase. Use
                    // `fetch_add` (not `store`) on `count` and
                    // `total_bytes` so concurrent matching
                    // writers never have their increments
                    // clobbered by our initial value. This lets
                    // the matching path skip `wait_published`
                    // entirely.
                    for i in 0..count {
                        bucket.sample_frames[i].store(frames.frames[i], Ordering::Relaxed);
                    }
                    bucket.count.fetch_add(1, Ordering::Relaxed);
                    bucket.total_bytes.fetch_add(size, Ordering::Relaxed);
                    bucket.frame_count.store(count as u64, Ordering::Release);
                    return;
                }
                Err(observed) => {
                    if observed == h {
                        // Same call site, another writer claimed
                        // first. The init thread now uses
                        // `fetch_add` (not `store`), so our
                        // increment cannot be clobbered even if
                        // it lands before init finishes.
                        bucket.count.fetch_add(1, Ordering::Relaxed);
                        bucket.total_bytes.fetch_add(size, Ordering::Relaxed);
                        return;
                    }
                    // Different site collided on this slot;
                    // probe forward.
                }
            }
        } else if existing == h {
            // Hot path: same site already in this bucket. No
            // wait_published needed because the init thread now
            // uses fetch_add for count/total_bytes; whether init
            // has finished publishing `sample_frames` is a
            // reader (`call_sites_report`) concern, not ours.
            bucket.count.fetch_add(1, Ordering::Relaxed);
            bucket.total_bytes.fetch_add(size, Ordering::Relaxed);
            return;
        }

        idx = (idx + 1) & mask;
        if idx == start {
            // Table full; drop event silently. Tracked in v0.9.2.
            return;
        }
    }
}

/// Drain the per-call-site table into a `Vec<CallSiteStats>`.
///
/// As of v0.9.6, events go straight from `record_event` into the
/// global table — there is no per-thread arena to flush first.
pub fn call_sites_report() -> Vec<CallSiteStats> {
    let Some((base, buckets, _mask)) = ensure_init() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for i in 0..buckets {
        // SAFETY: `i < buckets`, the bound used to allocate the
        // table, so `base.add(i)` is in-bounds.
        let bucket = unsafe { &*base.add(i) };
        let h = bucket.hash.load(Ordering::Acquire);
        if h == 0 {
            continue;
        }
        let fc = bucket.frame_count.load(Ordering::Acquire);
        if fc == 0 {
            // Claimed but not yet published; skip this snapshot.
            continue;
        }
        let n = (fc as usize).min(8);
        let mut frames = [0u64; 8];
        for (j, slot) in frames.iter_mut().enumerate().take(n) {
            *slot = bucket.sample_frames[j].load(Ordering::Relaxed);
        }
        out.push(CallSiteStats {
            frames,
            frame_count: n as u8,
            count: bucket.count.load(Ordering::Relaxed),
            total_bytes: bucket.total_bytes.load(Ordering::Relaxed),
        });
    }
    out
}

/// Reset the global table. Intended for tests only; production
/// callers should treat the table as monotonic.
#[doc(hidden)]
pub fn _reset_for_test() {
    let Some((base, buckets, _mask)) = ensure_init() else {
        return;
    };
    for i in 0..buckets {
        // SAFETY: bounds-checked by `buckets`, same justification
        // as `call_sites_report`.
        let bucket = unsafe { &*base.add(i) };
        bucket.hash.store(0, Ordering::Release);
        bucket.count.store(0, Ordering::Relaxed);
        bucket.total_bytes.store(0, Ordering::Relaxed);
        bucket.frame_count.store(0, Ordering::Release);
        for j in 0..8 {
            bucket.sample_frames[j].store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialise the table tests within this binary. They share
    // the process-wide aggregation table, and cargo runs unit
    // tests in parallel by default.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn records_and_reports_a_single_site() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        _reset_for_test();
        let frames = Frames {
            frames: [0xAAAA, 0xBBBB, 0xCCCC, 0, 0, 0, 0, 0],
            count: 3,
        };
        record(&frames, 100);
        record(&frames, 200);

        let report = call_sites_report();
        let site = report
            .iter()
            .find(|s| s.frames[0] == 0xAAAA && s.frames[1] == 0xBBBB)
            .expect("expected our site in the report");
        assert_eq!(site.frame_count, 3);
        assert_eq!(site.count, 2);
        assert_eq!(site.total_bytes, 300);
    }

    #[test]
    fn distinct_sites_are_separately_aggregated() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        _reset_for_test();
        let a = Frames {
            frames: [0xA000, 0xA001, 0, 0, 0, 0, 0, 0],
            count: 2,
        };
        let b = Frames {
            frames: [0xB000, 0xB001, 0, 0, 0, 0, 0, 0],
            count: 2,
        };
        for _ in 0..5 {
            record(&a, 10);
        }
        for _ in 0..3 {
            record(&b, 20);
        }
        let report = call_sites_report();
        let sa = report.iter().find(|s| s.frames[0] == 0xA000).unwrap();
        let sb = report.iter().find(|s| s.frames[0] == 0xB000).unwrap();
        assert_eq!(sa.count, 5);
        assert_eq!(sa.total_bytes, 50);
        assert_eq!(sb.count, 3);
        assert_eq!(sb.total_bytes, 60);
    }

    #[test]
    fn zero_frame_capture_is_ignored() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        _reset_for_test();
        let empty = Frames {
            frames: [0; 8],
            count: 0,
        };
        record(&empty, 50);
        let report = call_sites_report();
        assert!(
            report.iter().all(|s| s.frame_count > 0),
            "zero-frame capture should not appear"
        );
    }
}
