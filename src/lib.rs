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
//! [`ModAlloc::reset`] zeroes `alloc_count` and `total_bytes` and
//! sets the peaks to the current values; it never zeroes the live
//! counters, so it is safe with live allocations. [`Profiler`]
//! reports the peaks of its own window.
//!
//! The allocator path never panics: counter arithmetic wraps or
//! saturates instead of overflowing.
//!
//! ## Status
//!
//! **v1.x: stable API.** The public surface is frozen.
//! Breaking changes after `1.0.0` require a major version bump
//! per Semantic Versioning.
//!
//! Path to `1.0.0`:
//!
//! - `v0.9.0`: Real `GlobalAlloc` impl + Tier 1 counters.
//! - `v0.9.1`: Tier 2 inline backtrace capture (`backtraces`).
//! - `v0.9.2`: Symbolication for reports (`symbolicate`).
//! - `v0.9.3`: DHAT-compatible JSON output (`dhat-compat`).
//! - `v0.9.4`: `dhat_compat` drop-in surface for `dhat-rs`.
//! - `v0.9.5`: Tier 2 per-allocation overhead cut ~32x by
//!   removing the per-thread arena layer.
//! - `v1.0.0`: `#[non_exhaustive]` on the data structs likely
//!   to gain fields; public surface frozen.
//!
//! Default builds ship Tier 1 counters only.
//!
//! ## Backtraces (`backtraces` feature)
//!
//! With `mod-alloc = { version = "1", features = ["backtraces"] }`
//! and `RUSTFLAGS="-C force-frame-pointers=yes"`, each tracked
//! allocation captures up to 8 frames of its call site via inline
//! frame-pointer walking on `x86_64` and `aarch64`. Per-call-site
//! aggregation is exposed via `ModAlloc::call_sites` (available
//! only with the `backtraces` feature); the result is raw return
//! addresses; the `symbolicate` feature resolves them.
//!
//! Aggregation-table size is controlled by the `MOD_ALLOC_BUCKETS`
//! environment variable at process start (default 4,096 buckets,
//! ~384 KB).

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

#[cfg(feature = "backtraces")]
mod backtrace;

#[cfg(feature = "backtraces")]
pub use backtrace::CallSiteStats;

#[cfg(feature = "symbolicate")]
mod symbolicate;

#[cfg(feature = "symbolicate")]
pub use symbolicate::{SymbolicatedCallSite, SymbolicatedFrame};

#[cfg(feature = "dhat-compat")]
mod dhat_json;

#[cfg(feature = "dhat-compat")]
pub mod dhat_compat;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize};

// Number of per-thread counter stripes in each `ModAlloc`. Threads
// are assigned a stripe round-robin on their first tracked event, so
// up to this many threads update the cumulative counters without
// sharing a cache line.
const STRIPES: usize = 16;

// Number of profiling windows that can be open at once on one
// `ModAlloc`. A `Profiler` that finds every slot taken falls back to
// the lifetime peak.
const WINDOW_SLOTS: usize = 8;

// Process-wide handle to the `ModAlloc` that serves the installed
// `#[global_allocator]`. Set only by `installed_handle`, which finds
// the instance by sending one probe allocation through
// `std::alloc::alloc`: whichever `ModAlloc` receives that call is
// the global allocator, or is reached from it, and a
// `#[global_allocator]` item is always a `static`. A `ModAlloc` used
// directly (a local, or one inside a struct) is never recorded here,
// however many allocations it serves.
static GLOBAL_HANDLE: AtomicPtr<ModAlloc> = AtomicPtr::new(ptr::null_mut());

// Set once a probe has shown that the global allocator is not a
// `ModAlloc`, so later calls skip the probe.
static PROBED_NONE: AtomicBool = AtomicBool::new(false);

// Next stripe to hand out to a thread on its first tracked event.
static NEXT_STRIPE: AtomicUsize = AtomicUsize::new(0);

// Per-thread state used on the allocator path. `const`
// initialization of a type without `Drop` means access never
// allocates and never registers a destructor.
struct ThreadState {
    // Set while inside the tracking path; if any allocation occurs
    // while set, the recursive call bypasses tracking and forwards
    // directly to the System allocator.
    in_alloc: Cell<bool>,
    // Counter stripe for this thread, or `usize::MAX` before the
    // first tracked event.
    stripe: Cell<usize>,
    // Armed by `installed_handle` around its probe allocation.
    probing: Cell<bool>,
    // The `ModAlloc` that served the probe allocation, if any.
    probe_hit: Cell<*const ModAlloc>,
}

thread_local! {
    static STATE: ThreadState = const {
        ThreadState {
            in_alloc: Cell::new(false),
            stripe: Cell::new(usize::MAX),
            probing: Cell::new(false),
            probe_hit: Cell::new(ptr::null()),
        }
    };
}

// RAII guard for the reentrancy flag. `enter` returns `None` if the
// current thread is already inside a tracked allocation (caller
// must skip counter updates) or if TLS is unavailable (e.g. during
// thread teardown). The guard clears the flag on drop and carries
// the thread's counter stripe.
struct ReentryGuard {
    stripe: usize,
}

impl ReentryGuard {
    // `probe` is the allocator serving an allocation event (null for
    // a deallocation). If this thread is probing for the installed
    // allocator, the first such allocator is recorded.
    #[inline(always)]
    fn enter(probe: *const ModAlloc) -> Option<Self> {
        STATE
            .try_with(|s| {
                if !probe.is_null() && s.probing.get() && s.probe_hit.get().is_null() {
                    s.probe_hit.set(probe);
                }
                if s.in_alloc.get() {
                    return None;
                }
                s.in_alloc.set(true);
                let mut stripe = s.stripe.get();
                if stripe >= STRIPES {
                    stripe = NEXT_STRIPE.fetch_add(1, Ordering::Relaxed) % STRIPES;
                    s.stripe.set(stripe);
                }
                Some(ReentryGuard { stripe })
            })
            .ok()
            .flatten()
    }
}

impl Drop for ReentryGuard {
    fn drop(&mut self) {
        let _ = STATE.try_with(|s| s.in_alloc.set(false));
    }
}

/// Find the `ModAlloc` behind the installed `#[global_allocator]`.
///
/// Returns `None` when the global allocator is not a `ModAlloc` (and
/// does not forward to one). Must not be called from inside an
/// allocator.
pub(crate) fn installed_handle() -> Option<&'static ModAlloc> {
    let p = GLOBAL_HANDLE.load(Ordering::Acquire);
    if !p.is_null() {
        // SAFETY: `GLOBAL_HANDLE` is written only below, with the
        // address of the `ModAlloc` that served an allocation made
        // through `std::alloc::alloc`, that is, the instance behind
        // the `#[global_allocator]` item. That item is a `static`,
        // so the instance lives for the rest of the program. Only
        // shared access to atomic fields follows.
        return Some(unsafe { &*p });
    }
    if PROBED_NONE.load(Ordering::Acquire) {
        return None;
    }
    let armed = STATE
        .try_with(|s| {
            s.probing.set(true);
            s.probe_hit.set(ptr::null());
        })
        .is_ok();
    if !armed {
        return None;
    }
    let layout = Layout::new::<u8>();
    // SAFETY: `layout` has non-zero size. The block is freed right
    // away with the same layout.
    let block = unsafe { std::alloc::alloc(layout) };
    if !block.is_null() {
        // SAFETY: `block` came from `std::alloc::alloc(layout)`.
        unsafe { std::alloc::dealloc(block, layout) };
    }
    let hit = STATE
        .try_with(|s| {
            s.probing.set(false);
            s.probe_hit.replace(ptr::null())
        })
        .unwrap_or(ptr::null());
    if hit.is_null() {
        if !block.is_null() {
            PROBED_NONE.store(true, Ordering::Release);
        }
        return None;
    }
    let _ = GLOBAL_HANDLE.compare_exchange(
        ptr::null_mut(),
        hit as *mut ModAlloc,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    let p = GLOBAL_HANDLE.load(Ordering::Acquire);
    // SAFETY: as above; `p` is non-null because the exchange either
    // stored `hit` or found an earlier probe's result.
    Some(unsafe { &*p })
}

// Cumulative counters for the threads assigned to one stripe. Each
// stripe has its own cache line pair so threads do not contend.
#[repr(align(128))]
struct Stripe {
    alloc_count: AtomicU64,
    total_bytes: AtomicU64,
}

impl Stripe {
    // Only used as the repeat operand that builds the array in the
    // `const fn` constructor; each element is a fresh value.
    #[allow(clippy::declare_interior_mutable_const)]
    const NEW: Stripe = Stripe {
        alloc_count: AtomicU64::new(0),
        total_bytes: AtomicU64::new(0),
    };
}

// Live quantities, shared by all threads. Signed so that freeing a
// block this instance did not count (a `ModAlloc` used directly, not
// as the global allocator) cannot wrap.
#[repr(align(128))]
struct Live {
    current_bytes: AtomicI64,
    live_count: AtomicI64,
}

// Peaks, read on every event but written only when a new high is
// reached, so the line stays shared between cores.
#[repr(align(128))]
struct Peaks {
    peak_bytes: AtomicU64,
    peak_live_count: AtomicU64,
    // Bit `i` set while profiling window `i` is open.
    windows_active: AtomicU32,
}

struct Window {
    peak_bytes: AtomicU64,
    peak_live_count: AtomicU64,
}

impl Window {
    // See `Stripe::NEW`.
    #[allow(clippy::declare_interior_mutable_const)]
    const NEW: Window = Window {
        peak_bytes: AtomicU64::new(0),
        peak_live_count: AtomicU64::new(0),
    };
}

#[repr(align(128))]
struct Windows([Window; WINDOW_SLOTS]);

#[inline(always)]
fn clamp_non_negative(v: i64) -> u64 {
    if v > 0 {
        v as u64
    } else {
        0
    }
}

/// Global allocator wrapper that tracks allocations.
///
/// Install as `#[global_allocator]` to enable tracking. The wrapper
/// forwards every allocation, deallocation, reallocation, and
/// zero-initialised allocation to [`std::alloc::System`] and records
/// the event in lock-free atomic counters. Cumulative counters are
/// kept per thread stripe and summed when read, so threads do not
/// contend on them.
///
/// [`Profiler`] and `dhat_compat::HeapStats` read the instance that
/// serves the installed `#[global_allocator]`. A `ModAlloc` used
/// directly (for example a local wrapped by another allocator) keeps
/// its own counters, readable through [`ModAlloc::snapshot`], and is
/// never picked up by `Profiler`.
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
    stripes: [Stripe; STRIPES],
    live: Live,
    peaks: Peaks,
    windows: Windows,
    // `true` only for the allocator behind `dhat_compat::Alloc`: call
    // sites are then recorded only while a `dhat_compat::Profiler` is
    // alive, the only time that data is ever written out.
    #[cfg(feature = "dhat-compat")]
    sites_need_profiler: bool,
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
            stripes: [Stripe::NEW; STRIPES],
            live: Live {
                current_bytes: AtomicI64::new(0),
                live_count: AtomicI64::new(0),
            },
            peaks: Peaks {
                peak_bytes: AtomicU64::new(0),
                peak_live_count: AtomicU64::new(0),
                windows_active: AtomicU32::new(0),
            },
            windows: Windows([Window::NEW; WINDOW_SLOTS]),
            #[cfg(feature = "dhat-compat")]
            sites_need_profiler: false,
        }
    }

    /// Allocator for `dhat_compat::Alloc`: like [`ModAlloc::new`], but
    /// call sites are recorded only while a `dhat_compat::Profiler`
    /// is alive.
    #[cfg(feature = "dhat-compat")]
    pub(crate) const fn new_profiler_gated() -> Self {
        let mut a = Self::new();
        a.sites_need_profiler = true;
        a
    }

    /// Whether this allocation event should record its call site.
    #[cfg(feature = "backtraces")]
    #[inline(always)]
    fn records_sites(&self) -> bool {
        #[cfg(feature = "dhat-compat")]
        {
            !self.sites_need_profiler || dhat_compat::profiler_active()
        }
        #[cfg(not(feature = "dhat-compat"))]
        {
            true
        }
    }

    /// Snapshot the current counter values.
    ///
    /// Each counter is read independently with `Relaxed` ordering;
    /// the resulting [`AllocStats`] is a coherent best-effort view
    /// but does not represent a single atomic moment in time. For
    /// scoped measurement, prefer [`Profiler`].
    ///
    /// `current_bytes` and `live_count` never read below zero, and
    /// the peaks never read below the current values.
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
        let mut alloc_count: u64 = 0;
        let mut total_bytes: u64 = 0;
        for s in &self.stripes {
            alloc_count = alloc_count.wrapping_add(s.alloc_count.load(Ordering::Relaxed));
            total_bytes = total_bytes.wrapping_add(s.total_bytes.load(Ordering::Relaxed));
        }
        let current_bytes = clamp_non_negative(self.live.current_bytes.load(Ordering::Relaxed));
        let live_count = clamp_non_negative(self.live.live_count.load(Ordering::Relaxed));
        AllocStats {
            alloc_count,
            total_bytes,
            peak_bytes: self
                .peaks
                .peak_bytes
                .load(Ordering::Relaxed)
                .max(current_bytes),
            current_bytes,
            live_count,
            peak_live_count: self
                .peaks
                .peak_live_count
                .load(Ordering::Relaxed)
                .max(live_count),
        }
    }

    /// Start a new measurement period.
    ///
    /// Zeroes the cumulative counters (`alloc_count`, `total_bytes`)
    /// and sets each peak to the current value, so later peaks
    /// describe the period after the call. `current_bytes` and
    /// `live_count` are not changed: memory that is still allocated
    /// stays counted, and freeing it later lowers them as usual.
    /// It is safe to call at any time, with any number of live
    /// allocations.
    ///
    /// A [`Profiler`] that is running across a `reset` reports
    /// cumulative deltas of zero for the part of its window before
    /// the call; prefer `Profiler` for scoped measurement.
    ///
    /// Before 1.1.0 this zeroed every counter, which made the live
    /// counters wrap when an older block was freed.
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
        for s in &self.stripes {
            s.alloc_count.store(0, Ordering::Relaxed);
            s.total_bytes.store(0, Ordering::Relaxed);
        }
        let current = clamp_non_negative(self.live.current_bytes.load(Ordering::Relaxed));
        let live = clamp_non_negative(self.live.live_count.load(Ordering::Relaxed));
        self.peaks.peak_bytes.store(current, Ordering::Relaxed);
        self.peaks.peak_live_count.store(live, Ordering::Relaxed);
    }

    // Raise the lifetime peaks and any open window peaks to the
    // given values. Loads first so that, in the steady state, the
    // peak line is only read.
    #[inline(always)]
    fn raise_peaks(&self, current: u64, live: u64) {
        if current > self.peaks.peak_bytes.load(Ordering::Relaxed) {
            self.peaks.peak_bytes.fetch_max(current, Ordering::Relaxed);
        }
        if live > self.peaks.peak_live_count.load(Ordering::Relaxed) {
            self.peaks
                .peak_live_count
                .fetch_max(live, Ordering::Relaxed);
        }
        let mask = self.peaks.windows_active.load(Ordering::Relaxed);
        if mask != 0 {
            self.raise_windows(mask, current, live);
        }
    }

    #[cold]
    #[inline(never)]
    fn raise_windows(&self, mut mask: u32, current: u64, live: u64) {
        while mask != 0 {
            let i = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            if let Some(w) = self.windows.0.get(i) {
                if current > w.peak_bytes.load(Ordering::Relaxed) {
                    w.peak_bytes.fetch_max(current, Ordering::Relaxed);
                }
                if live > w.peak_live_count.load(Ordering::Relaxed) {
                    w.peak_live_count.fetch_max(live, Ordering::Relaxed);
                }
            }
        }
    }

    #[inline]
    fn record_alloc(&self, stripe: usize, size: u64) {
        let s = &self.stripes[stripe % STRIPES];
        s.alloc_count.fetch_add(1, Ordering::Relaxed);
        s.total_bytes.fetch_add(size, Ordering::Relaxed);
        // `size` comes from a `Layout`, so it is at most `isize::MAX`.
        let size = size as i64;
        let current = self
            .live
            .current_bytes
            .fetch_add(size, Ordering::Relaxed)
            .wrapping_add(size);
        let live = self
            .live
            .live_count
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        self.raise_peaks(clamp_non_negative(current), clamp_non_negative(live));
    }

    #[inline]
    fn record_dealloc(&self, size: u64) {
        self.live
            .current_bytes
            .fetch_sub(size as i64, Ordering::Relaxed);
        self.live.live_count.fetch_sub(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_realloc(&self, stripe: usize, old_size: u64, new_size: u64) {
        let s = &self.stripes[stripe % STRIPES];
        s.alloc_count.fetch_add(1, Ordering::Relaxed);
        match new_size.cmp(&old_size) {
            std::cmp::Ordering::Greater => {
                let delta = new_size - old_size;
                s.total_bytes.fetch_add(delta, Ordering::Relaxed);
                let delta = delta as i64;
                let current = self
                    .live
                    .current_bytes
                    .fetch_add(delta, Ordering::Relaxed)
                    .wrapping_add(delta);
                self.raise_peaks(clamp_non_negative(current), 0);
            }
            std::cmp::Ordering::Less => {
                self.live
                    .current_bytes
                    .fetch_sub((old_size - new_size) as i64, Ordering::Relaxed);
            }
            std::cmp::Ordering::Equal => {}
        }
    }

    // Open a profiling window: claim a free slot and start its peaks
    // at the current values. `None` when every slot is in use.
    pub(crate) fn open_window(&self) -> Option<usize> {
        let mut mask = self.peaks.windows_active.load(Ordering::Relaxed);
        loop {
            let free = (!mask).trailing_zeros() as usize;
            if free >= WINDOW_SLOTS {
                return None;
            }
            let bit = 1u32 << free;
            match self.peaks.windows_active.compare_exchange_weak(
                mask,
                mask | bit,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let w = &self.windows.0[free];
                    w.peak_bytes.store(0, Ordering::Relaxed);
                    w.peak_live_count.store(0, Ordering::Relaxed);
                    let current =
                        clamp_non_negative(self.live.current_bytes.load(Ordering::Relaxed));
                    let live = clamp_non_negative(self.live.live_count.load(Ordering::Relaxed));
                    w.peak_bytes.fetch_max(current, Ordering::Relaxed);
                    w.peak_live_count.fetch_max(live, Ordering::Relaxed);
                    return Some(free);
                }
                Err(seen) => mask = seen,
            }
        }
    }

    // Peaks of an open window: (bytes, live blocks).
    pub(crate) fn window_peaks(&self, slot: usize) -> (u64, u64) {
        match self.windows.0.get(slot) {
            Some(w) => (
                w.peak_bytes.load(Ordering::Relaxed),
                w.peak_live_count.load(Ordering::Relaxed),
            ),
            None => (0, 0),
        }
    }

    pub(crate) fn close_window(&self, slot: usize) {
        if slot < WINDOW_SLOTS {
            self.peaks
                .windows_active
                .fetch_and(!(1u32 << slot), Ordering::AcqRel);
        }
    }

    /// Drain the per-call-site aggregation table into a `Vec`.
    ///
    /// Available only with the `backtraces` cargo feature. The
    /// returned vector contains one [`CallSiteStats`] per unique
    /// call site observed since the table was first written. Each
    /// row carries up to 8 raw return addresses (top of stack
    /// first), the number of allocations attributed to that site,
    /// and the total bytes.
    ///
    /// This method exposes raw addresses only; see
    /// `symbolicated_report` (feature `symbolicate`) for function
    /// names.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(feature = "backtraces")]
    /// # fn demo() {
    /// use mod_alloc::ModAlloc;
    ///
    /// #[global_allocator]
    /// static GLOBAL: ModAlloc = ModAlloc::new();
    ///
    /// let _v: Vec<u8> = vec![0; 1024];
    /// for site in GLOBAL.call_sites() {
    ///     println!("{} allocs, {} bytes at {:#x}",
    ///         site.count, site.total_bytes, site.frames[0]);
    /// }
    /// # }
    /// ```
    #[cfg(feature = "backtraces")]
    pub fn call_sites(&self) -> Vec<CallSiteStats> {
        backtrace::call_sites_report()
    }

    /// Drain the per-call-site table and symbolicate each frame
    /// against the running binary's own debug info.
    ///
    /// Available only with the `symbolicate` cargo feature, which
    /// also implies `backtraces`. Returns one
    /// [`SymbolicatedCallSite`] per unique call site, each
    /// carrying resolved function names plus (where available)
    /// source file and line.
    ///
    /// Allocates. Safe to call from non-allocator contexts only
    /// (ordinary user code outside the global-allocator hook).
    ///
    /// Results are cached per-address across calls.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(feature = "symbolicate")]
    /// # fn demo() {
    /// use mod_alloc::ModAlloc;
    ///
    /// #[global_allocator]
    /// static GLOBAL: ModAlloc = ModAlloc::new();
    ///
    /// let _v: Vec<u8> = vec![0; 1024];
    /// for site in GLOBAL.symbolicated_report() {
    ///     let top = &site.frames[0];
    ///     println!("{} allocs / {} bytes at {}",
    ///         site.count,
    ///         site.total_bytes,
    ///         top.function.as_deref().unwrap_or("<unresolved>"));
    /// }
    /// # }
    /// ```
    #[cfg(feature = "symbolicate")]
    pub fn symbolicated_report(&self) -> Vec<SymbolicatedCallSite> {
        symbolicate::symbolicated_report()
    }

    /// Render the per-call-site report as a DHAT-compatible JSON
    /// string.
    ///
    /// Available only with the `dhat-compat` cargo feature (which
    /// implies `backtraces`). When the `symbolicate` feature is
    /// also active, frame strings carry function names and
    /// (where available) source file and line; otherwise they
    /// carry raw hex addresses.
    ///
    /// Allocates. Safe to call from non-allocator contexts only
    /// (ordinary user code outside the global-allocator hook).
    ///
    /// The output schema (`dhatFileVersion: 2`, `mode: "rust-heap"`)
    /// matches the format consumed by the upstream
    /// `dh_view.html` viewer shipped with Valgrind.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(feature = "dhat-compat")]
    /// # fn demo() {
    /// use mod_alloc::ModAlloc;
    ///
    /// #[global_allocator]
    /// static GLOBAL: ModAlloc = ModAlloc::new();
    ///
    /// let _v: Vec<u8> = vec![0; 1024];
    /// let json = GLOBAL.dhat_json_string();
    /// assert!(json.contains("\"dhatFileVersion\":2"));
    /// # }
    /// ```
    #[cfg(feature = "dhat-compat")]
    pub fn dhat_json_string(&self) -> String {
        dhat_json::dhat_json_string()
    }

    /// Render the per-call-site report and write it to `path` as
    /// DHAT-compatible JSON.
    ///
    /// Available only with the `dhat-compat` cargo feature.
    /// Mirrors `dhat-rs`'s convention of writing
    /// `dhat-heap.json` to the current directory; pass that path
    /// to drop a file the upstream viewer will load directly.
    ///
    /// Allocates. Safe to call from non-allocator contexts only.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(feature = "dhat-compat")]
    /// # fn demo() -> std::io::Result<()> {
    /// use mod_alloc::ModAlloc;
    ///
    /// #[global_allocator]
    /// static GLOBAL: ModAlloc = ModAlloc::new();
    ///
    /// let _v: Vec<u8> = vec![0; 1024];
    /// GLOBAL.write_dhat_json("dhat-heap.json")?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "dhat-compat")]
    pub fn write_dhat_json<P: AsRef<std::path::Path>>(&self, path: P) -> std::io::Result<()> {
        dhat_json::write_dhat_json(path.as_ref())
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
            if let Some(g) = ReentryGuard::enter(self) {
                let size = layout.size() as u64;
                self.record_alloc(g.stripe, size);
                #[cfg(feature = "backtraces")]
                if self.records_sites() {
                    backtrace::record_event(size);
                }
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
            if let Some(g) = ReentryGuard::enter(self) {
                let size = layout.size() as u64;
                self.record_alloc(g.stripe, size);
                #[cfg(feature = "backtraces")]
                if self.records_sites() {
                    backtrace::record_event(size);
                }
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
        if let Some(_g) = ReentryGuard::enter(ptr::null()) {
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
            if let Some(g) = ReentryGuard::enter(self) {
                self.record_realloc(g.stripe, layout.size() as u64, new_size as u64);
                // Per dhat semantics: realloc records as one event
                // attributed to `new_size` (including shrinks).
                #[cfg(feature = "backtraces")]
                if self.records_sites() {
                    backtrace::record_event(new_size as u64);
                }
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
/// // Construct via Default (literal construction is closed off as
/// // of v1.0 — see "Stability" below).
/// let mut stats = AllocStats::default();
/// stats.alloc_count = 10;
/// stats.total_bytes = 1024;
/// assert_eq!(stats.alloc_count, 10);
/// ```
///
/// # Stability
///
/// Marked `#[non_exhaustive]` as of v1.0.0. New counter fields may
/// be added in future minor versions without bumping the major
/// version. Construct via [`AllocStats::default`] (or consume
/// snapshots from [`ModAlloc::snapshot`] / [`Profiler::stop`])
/// rather than struct-literal syntax. Reading individual fields by
/// name is fully stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct AllocStats {
    /// Number of allocations performed.
    pub alloc_count: u64,
    /// Total bytes allocated across all allocations. Reallocations
    /// contribute the growth delta (or zero on shrink).
    pub total_bytes: u64,
    /// Peak resident bytes: the highest `current_bytes` observed
    /// since counting began or the last [`ModAlloc::reset`]. From
    /// [`Profiler::stop`], the highest during the profiling window.
    pub peak_bytes: u64,
    /// Currently-allocated bytes (allocations minus deallocations).
    pub current_bytes: u64,
    /// Currently-alive allocation count (allocations minus
    /// deallocations). Mirrors `dhat::HeapStats::curr_blocks`.
    pub live_count: u64,
    /// Peak live allocation count: the highest `live_count`
    /// observed, over the same period as `peak_bytes`. Mirrors
    /// `dhat::HeapStats::max_blocks`.
    pub peak_live_count: u64,
}

/// Scoped profiler that captures a delta between start and stop.
///
/// Reads the [`ModAlloc`] behind the installed `#[global_allocator]`
/// on construction and again on [`Profiler::stop`], returning the
/// difference. If the global allocator is not a `ModAlloc`, both
/// snapshots are zero and the delta is zero.
///
/// Each `Profiler` tracks its own peak, so nested and concurrent
/// profilers each report the high-water mark of their own window.
/// Up to 8 can be open at once; a ninth reports the lifetime peak
/// instead.
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
    handle: Option<&'static ModAlloc>,
    window: Option<usize>,
}

impl Profiler {
    /// Begin profiling, capturing the current allocation state.
    ///
    /// The first call finds the installed allocator with one small
    /// probe allocation, made before the baseline is taken. If the
    /// global allocator is not a `ModAlloc`, the captured baseline
    /// is all zeros.
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
        let handle = installed_handle();
        let window = handle.and_then(ModAlloc::open_window);
        let baseline = handle.map_or_else(zero_stats, ModAlloc::snapshot);
        Self {
            baseline,
            handle,
            window,
        }
    }

    /// Stop profiling and return the delta from start.
    ///
    /// `alloc_count`, `total_bytes`, `current_bytes` and `live_count`
    /// are deltas from `start()` to `stop()`. `peak_bytes` and
    /// `peak_live_count` are the absolute high-water marks reached
    /// during the profiling window, including what was already
    /// allocated at `start()`. (Before 1.1.0 they were the lifetime
    /// peaks.)
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
    pub fn stop(mut self) -> AllocStats {
        let Some(handle) = self.handle else {
            return zero_stats();
        };
        let now = handle.snapshot();
        let (peak_bytes, peak_live_count) = match self.window.take() {
            Some(slot) => {
                let peaks = handle.window_peaks(slot);
                handle.close_window(slot);
                peaks
            }
            None => (now.peak_bytes, now.peak_live_count),
        };
        AllocStats {
            alloc_count: now.alloc_count.saturating_sub(self.baseline.alloc_count),
            total_bytes: now.total_bytes.saturating_sub(self.baseline.total_bytes),
            current_bytes: now
                .current_bytes
                .saturating_sub(self.baseline.current_bytes),
            peak_bytes: peak_bytes.max(now.current_bytes),
            live_count: now.live_count.saturating_sub(self.baseline.live_count),
            peak_live_count: peak_live_count.max(now.live_count),
        }
    }
}

impl Drop for Profiler {
    fn drop(&mut self) {
        if let (Some(handle), Some(slot)) = (self.handle, self.window.take()) {
            handle.close_window(slot);
        }
    }
}

pub(crate) fn zero_stats() -> AllocStats {
    AllocStats {
        alloc_count: 0,
        total_bytes: 0,
        peak_bytes: 0,
        current_bytes: 0,
        live_count: 0,
        peak_live_count: 0,
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
        a.record_alloc(0, 128);
        a.record_alloc(0, 256);
        let s = a.snapshot();
        assert_eq!(s.alloc_count, 2);
        assert_eq!(s.total_bytes, 384);
        assert_eq!(s.current_bytes, 384);
        assert_eq!(s.peak_bytes, 384);
    }

    #[test]
    fn record_dealloc_decreases_current_only() {
        let a = ModAlloc::new();
        a.record_alloc(0, 1000);
        a.record_dealloc(400);
        let s = a.snapshot();
        assert_eq!(s.alloc_count, 1);
        assert_eq!(s.total_bytes, 1000);
        assert_eq!(s.current_bytes, 600);
        assert_eq!(s.peak_bytes, 1000);
        // live_count tracks the *count* of alive allocations, not
        // bytes — a single alloc + a single (partial) dealloc
        // event leaves zero alive allocations from the counter's
        // perspective even though current_bytes is non-zero.
        // This mirrors dhat's curr_blocks semantics where every
        // alloc/dealloc pair is a 1:1 block lifecycle.
        assert_eq!(s.live_count, 0);
        assert_eq!(s.peak_live_count, 1);
    }

    #[test]
    fn live_counters_track_alive_blocks() {
        let a = ModAlloc::new();
        a.record_alloc(0, 100);
        a.record_alloc(0, 200);
        a.record_alloc(0, 300);
        let s = a.snapshot();
        assert_eq!(s.live_count, 3);
        assert_eq!(s.peak_live_count, 3);

        a.record_dealloc(100);
        a.record_dealloc(200);
        let s = a.snapshot();
        assert_eq!(s.live_count, 1);
        assert_eq!(s.peak_live_count, 3, "peak holds high-water mark");
    }

    #[test]
    fn record_realloc_does_not_touch_live_count() {
        let a = ModAlloc::new();
        a.record_alloc(0, 100);
        let pre = a.snapshot();
        a.record_realloc(0, 100, 250);
        let post = a.snapshot();
        assert_eq!(
            pre.live_count, post.live_count,
            "realloc keeps the same block, live_count unchanged"
        );
    }

    #[test]
    fn record_realloc_growth_updates_total_and_peak() {
        let a = ModAlloc::new();
        a.record_alloc(0, 100);
        a.record_realloc(0, 100, 250);
        let s = a.snapshot();
        assert_eq!(s.alloc_count, 2);
        assert_eq!(s.total_bytes, 250);
        assert_eq!(s.current_bytes, 250);
        assert_eq!(s.peak_bytes, 250);
    }

    #[test]
    fn record_realloc_shrink_only_adjusts_current() {
        let a = ModAlloc::new();
        a.record_alloc(0, 500);
        a.record_realloc(0, 500, 200);
        let s = a.snapshot();
        assert_eq!(s.alloc_count, 2);
        assert_eq!(s.total_bytes, 500);
        assert_eq!(s.current_bytes, 200);
        assert_eq!(s.peak_bytes, 500);
    }

    #[test]
    fn peak_holds_high_water_mark() {
        let a = ModAlloc::new();
        a.record_alloc(0, 1000);
        a.record_dealloc(1000);
        a.record_alloc(0, 500);
        let s = a.snapshot();
        assert_eq!(s.peak_bytes, 1000);
        assert_eq!(s.current_bytes, 500);
    }

    #[test]
    fn dealloc_of_uncounted_block_does_not_wrap() {
        // A `ModAlloc` used directly can free blocks it never counted.
        let a = ModAlloc::new();
        a.record_dealloc(64);
        let s = a.snapshot();
        assert_eq!(s.current_bytes, 0);
        assert_eq!(s.live_count, 0);
        a.record_alloc(0, 32);
        let s = a.snapshot();
        assert!(s.current_bytes <= 32, "{s:?}");
        assert!(s.peak_bytes <= 32, "{s:?}");
    }

    #[test]
    fn reset_rebases_instead_of_zeroing() {
        let a = ModAlloc::new();
        a.record_alloc(0, 1000);
        a.record_alloc(1, 500);
        a.record_dealloc(500);
        a.reset();
        let s = a.snapshot();
        assert_eq!(s.alloc_count, 0);
        assert_eq!(s.total_bytes, 0);
        assert_eq!(s.current_bytes, 1000);
        assert_eq!(s.live_count, 1);
        assert_eq!(s.peak_bytes, 1000);
        assert_eq!(s.peak_live_count, 1);
        a.record_dealloc(1000);
        a.record_alloc(2, 10);
        let s = a.snapshot();
        assert_eq!(s.current_bytes, 10);
        assert_eq!(s.live_count, 1);
        assert_eq!(s.peak_bytes, 1000);
        assert_eq!(s.alloc_count, 1);
    }

    #[test]
    fn stripes_are_summed() {
        let a = ModAlloc::new();
        for stripe in 0..STRIPES * 2 {
            a.record_alloc(stripe, 8);
        }
        a.record_realloc(STRIPES - 1, 8, 24);
        let s = a.snapshot();
        assert_eq!(s.alloc_count, (STRIPES * 2 + 1) as u64);
        assert_eq!(s.total_bytes, (STRIPES * 2 * 8 + 16) as u64);
        assert_eq!(s.current_bytes, (STRIPES * 2 * 8 + 16) as u64);
    }

    #[test]
    fn windows_track_their_own_peak() {
        let a = ModAlloc::new();
        a.record_alloc(0, 5000);
        a.record_dealloc(5000);
        a.record_alloc(0, 100);
        let w1 = a.open_window().unwrap();
        a.record_alloc(0, 300);
        let w2 = a.open_window().unwrap();
        a.record_dealloc(300);
        a.record_alloc(0, 50);
        assert_eq!(a.window_peaks(w1), (400, 2));
        assert_eq!(a.window_peaks(w2), (400, 2));
        a.close_window(w2);
        a.record_alloc(0, 1000);
        assert_eq!(a.window_peaks(w1).0, 1150);
        assert_eq!(a.window_peaks(w2).0, 400, "closed window is frozen");
        a.close_window(w1);
        assert_eq!(a.snapshot().peak_bytes, 5000);
    }

    #[test]
    fn concurrent_records_sum_exactly() {
        const THREADS: usize = 4;
        const EVENTS: usize = 200;
        let a = ModAlloc::new();
        let w = a.open_window().unwrap();
        std::thread::scope(|s| {
            for t in 0..THREADS {
                let a = &a;
                s.spawn(move || {
                    for _ in 0..EVENTS {
                        a.record_alloc(t, 16);
                        a.record_realloc(t, 16, 32);
                        a.record_dealloc(32);
                    }
                });
            }
        });
        let s = a.snapshot();
        assert_eq!(s.alloc_count, (THREADS * EVENTS * 2) as u64);
        assert_eq!(s.total_bytes, (THREADS * EVENTS * 32) as u64);
        assert_eq!(s.current_bytes, 0);
        assert_eq!(s.live_count, 0);
        let (peak, peak_live) = a.window_peaks(w);
        assert!((32..=(THREADS as u64) * 32).contains(&peak), "{peak}");
        assert!((1..=THREADS as u64).contains(&peak_live), "{peak_live}");
        a.close_window(w);
    }

    #[test]
    fn window_slots_run_out_and_recover() {
        let a = ModAlloc::new();
        let slots: Vec<usize> = (0..WINDOW_SLOTS)
            .map(|_| a.open_window().unwrap())
            .collect();
        assert!(a.open_window().is_none());
        a.close_window(slots[3]);
        assert_eq!(a.open_window(), Some(slots[3]));
    }

    #[test]
    fn reentry_guard_blocks_nested_entry() {
        let outer = ReentryGuard::enter(ptr::null());
        assert!(outer.is_some());
        let inner = ReentryGuard::enter(ptr::null());
        assert!(inner.is_none(), "nested entry must be denied");
        drop(outer);
        let after = ReentryGuard::enter(ptr::null());
        assert!(after.is_some(), "entry must be allowed after outer drops");
    }

    #[test]
    fn profiler_start_stop_with_no_handle() {
        let p = Profiler::start();
        let s = p.stop();
        assert_eq!(s.alloc_count, 0);
    }
}
