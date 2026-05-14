//! Joins raw `CallSiteStats` rows to `SymbolicatedCallSite` rows
//! by resolving each frame address against the platform
//! symbolicator. Maintains a per-process `address -> Vec<Frame>`
//! cache so repeat calls to `symbolicated_report()` reuse the
//! resolution work.

use std::collections::HashMap;
use std::sync::Mutex;

use super::{SymbolicatedCallSite, SymbolicatedFrame};

static ADDRESS_CACHE: Mutex<Option<HashMap<u64, Vec<SymbolicatedFrame>>>> = Mutex::new(None);

fn resolve_one(address: u64) -> Vec<SymbolicatedFrame> {
    #[cfg(unix)]
    {
        super::unix::resolve(address)
    }
    #[cfg(windows)]
    {
        super::windows::resolve(address)
    }
    #[cfg(not(any(unix, windows)))]
    {
        vec![SymbolicatedFrame {
            address,
            function: None,
            file: None,
            line: None,
            inlined: false,
        }]
    }
}

fn cached_resolve(address: u64) -> Vec<SymbolicatedFrame> {
    if address == 0 {
        return vec![];
    }
    // Fast path: cache hit. Clone the cached entry so the lock is
    // released quickly.
    {
        let mut guard = ADDRESS_CACHE.lock().unwrap_or_else(|p| p.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        if let Some(frames) = map.get(&address) {
            return frames.clone();
        }
    }

    // Slow path: resolve then memoise.
    let frames = resolve_one(address);
    let mut guard = ADDRESS_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    map.entry(address).or_insert_with(|| frames.clone());
    frames
}

/// Drain the raw report and resolve each frame address.
///
/// Allocates. Safe to call from non-allocator contexts only
/// (i.e. ordinary user code outside the global-allocator hook).
pub fn symbolicated_report() -> Vec<SymbolicatedCallSite> {
    let raw = crate::backtrace::call_sites_report();
    let mut out = Vec::with_capacity(raw.len());

    for row in raw {
        let mut frames: Vec<SymbolicatedFrame> = Vec::new();
        for i in 0..(row.frame_count as usize) {
            let addr = row.frames[i];
            let resolved = cached_resolve(addr);
            frames.extend(resolved);
        }
        out.push(SymbolicatedCallSite {
            count: row.count,
            total_bytes: row.total_bytes,
            frames,
        });
    }

    out
}

/// Clear the per-process address cache. Intended for tests; in
/// production the cache is monotonic.
#[doc(hidden)]
pub fn _reset_cache_for_test() {
    let mut guard = ADDRESS_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    *guard = None;
}
