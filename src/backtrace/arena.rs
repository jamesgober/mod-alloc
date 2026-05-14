//! Per-thread arena for backtrace events.
//!
//! Each thread holds a pointer to a 40 KB region (512 entries
//! × 80 bytes) inside a 64 KB OS-allocated page run, lazily
//! created on the first event recorded by that thread. Events
//! append at the cursor; when the arena fills, it flushes
//! synchronously into the global aggregation table. The arena
//! also flushes on thread exit via a `Drop` impl on the TLS slot.
//!
//! All storage comes from raw OS pages (see [`super::raw_mem`]),
//! never from `ModAlloc` itself; otherwise the allocator hook
//! would recurse into the very state it is trying to populate.

use core::cell::UnsafeCell;
use core::ptr;

use super::raw_mem::{alloc_pages, free_pages};
use super::table;
use super::walk::Frames;

/// Number of entries in the per-thread arena before a flush.
pub(crate) const ENTRIES_PER_ARENA: usize = 512;

/// Size of the OS-allocated region per thread.
const ARENA_BYTES: usize = 64 * 1024;

#[repr(C)]
#[derive(Clone, Copy)]
struct Entry {
    frames: [u64; 8],
    size: u64,
    frame_count: u64,
}

struct ArenaState {
    base: *mut Entry,
    cursor: usize,
}

impl ArenaState {
    const fn new() -> Self {
        Self {
            base: ptr::null_mut(),
            cursor: 0,
        }
    }

    fn ensure_init(&mut self) -> bool {
        if !self.base.is_null() {
            return true;
        }
        // SAFETY: alloc_pages contract: returns null on failure
        // or a writable, zero-init region of the requested size.
        let pages = unsafe { alloc_pages(ARENA_BYTES) };
        if pages.is_null() {
            return false;
        }
        self.base = pages as *mut Entry;
        self.cursor = 0;
        true
    }

    fn append(&mut self, frames: &Frames, size: u64) {
        if self.cursor >= ENTRIES_PER_ARENA {
            self.flush();
        }
        // SAFETY: `self.base` is the start of an array of
        // `ENTRIES_PER_ARENA` entries inside a region of
        // `ARENA_BYTES`. After the flush above, `self.cursor`
        // is strictly less than `ENTRIES_PER_ARENA`, so the
        // computed pointer is in-bounds for a write.
        unsafe {
            let slot = self.base.add(self.cursor);
            (*slot).frames = frames.frames;
            (*slot).size = size;
            (*slot).frame_count = frames.count as u64;
        }
        self.cursor += 1;
    }

    fn flush(&mut self) {
        if self.base.is_null() || self.cursor == 0 {
            self.cursor = 0;
            return;
        }
        let count = self.cursor;
        // SAFETY: `self.base` is a valid pointer to at least
        // `count` initialised `Entry` values; we only read,
        // never alias.
        unsafe {
            for i in 0..count {
                let e = &*self.base.add(i);
                let mut fr = Frames {
                    frames: e.frames,
                    count: e.frame_count as u8,
                };
                if (fr.count as usize) > fr.frames.len() {
                    fr.count = fr.frames.len() as u8;
                }
                table::record(&fr, e.size);
            }
        }
        self.cursor = 0;
    }

    fn release(&mut self) {
        if self.base.is_null() {
            return;
        }
        self.flush();
        // SAFETY: `self.base` was returned by `alloc_pages` with
        // size `ARENA_BYTES`. We pair the call exactly.
        unsafe {
            free_pages(self.base as *mut u8, ARENA_BYTES);
        }
        self.base = ptr::null_mut();
        self.cursor = 0;
    }
}

/// TLS slot wrapper. `Drop` flushes the arena and releases its
/// pages when the owning thread exits.
struct ArenaSlot {
    state: UnsafeCell<ArenaState>,
}

impl ArenaSlot {
    const fn new() -> Self {
        Self {
            state: UnsafeCell::new(ArenaState::new()),
        }
    }
}

impl Drop for ArenaSlot {
    fn drop(&mut self) {
        // SAFETY: `state` is only ever borrowed mutably inside
        // calls invoked while the owning thread is alive. By the
        // time `Drop` runs, no other code on this thread can
        // observe the slot.
        unsafe {
            let state = &mut *self.state.get();
            state.release();
        }
    }
}

thread_local! {
    static ARENA: ArenaSlot = const { ArenaSlot::new() };
}

/// Record a backtrace event in this thread's arena. Flushes the
/// arena into the global table if full. Silent no-op if TLS is
/// unavailable or the OS-page allocation failed.
pub(crate) fn record_event(frames: &Frames, size: u64) {
    let _ = ARENA.try_with(|slot| {
        // SAFETY: `state` is only ever mutated from this thread
        // (TLS storage). The borrow lives only across this
        // closure body.
        unsafe {
            let state = &mut *slot.state.get();
            if !state.ensure_init() {
                return;
            }
            state.append(frames, size);
        }
    });
}

/// Force any pending entries in this thread's arena out to the
/// global aggregation table. Intended for tests and for
/// report-time accuracy.
pub(crate) fn flush_current_thread() {
    let _ = ARENA.try_with(|slot| {
        // SAFETY: same justification as `record_event`.
        unsafe {
            let state = &mut *slot.state.get();
            state.flush();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_flush_round_trip() {
        let frames = Frames {
            frames: [0x1000, 0x2000, 0x3000, 0, 0, 0, 0, 0],
            count: 3,
        };
        record_event(&frames, 64);
        flush_current_thread();
        // The table received an event; no panic means success.
        // (Detailed table behaviour is exercised in table.rs tests.)
    }
}
