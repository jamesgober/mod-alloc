//! Per-thread stack-bounds discovery.
//!
//! The frame-pointer walker requires the current thread's stack
//! range to validate that each frame pointer it dereferences lies
//! inside legitimate stack memory. This module exposes a single
//! function `current_stack_bounds()` that queries the OS and
//! caches the answer in thread-local storage.
//!
//! - **Windows:** `GetCurrentThreadStackLimits` (Vista+).
//! - **Linux:** `pthread_getattr_np` + `pthread_attr_getstack`.
//! - **macOS / iOS / *BSD:** `pthread_get_stackaddr_np`
//!   + `pthread_get_stacksize_np`.
//! - **Other targets:** the function returns `None`. The walker
//!   then refuses to dereference any FP and returns zero frames.
//!
//! All foreign declarations are inline `extern "C"` / `extern
//! "system"` blocks; no `libc` crate dependency per the
//! zero-runtime-deps policy.

use std::cell::Cell;

/// Inclusive bounds of the current thread's stack: `low..=high - 16`.
/// Stacks grow from `high` toward `low` on supported targets.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StackBounds {
    pub(crate) low: usize,
    pub(crate) high: usize,
}

thread_local! {
    static CACHED: Cell<Option<StackBounds>> = const { Cell::new(None) };
}

/// Return the current thread's stack bounds, querying the OS on
/// the first call and returning the cached value on subsequent
/// calls. Returns `None` if the platform is unsupported or the
/// query failed.
///
/// Marked `#[inline(always)]` so the cache-hit fast path folds
/// into the calling `record_event` body without a function-call
/// boundary; the OS query is itself out-of-line via the cfg-
/// specific `query_os` helper, which only runs on the first call
/// per thread.
#[inline(always)]
pub(crate) fn current_stack_bounds() -> Option<StackBounds> {
    if let Ok(cached) = CACHED.try_with(|c| c.get()) {
        if let Some(b) = cached {
            return Some(b);
        }
        let fresh = query_os();
        if let Some(b) = fresh {
            let _ = CACHED.try_with(|c| c.set(Some(b)));
        }
        fresh
    } else {
        query_os()
    }
}

#[cfg(target_os = "windows")]
fn query_os() -> Option<StackBounds> {
    extern "system" {
        fn GetCurrentThreadStackLimits(low_limit: *mut usize, high_limit: *mut usize);
    }
    let mut low: usize = 0;
    let mut high: usize = 0;
    // SAFETY: GetCurrentThreadStackLimits writes two usize values.
    // The pointers are valid for the duration of the call.
    unsafe {
        GetCurrentThreadStackLimits(&mut low, &mut high);
    }
    if low == 0 || high == 0 || high <= low {
        None
    } else {
        Some(StackBounds { low, high })
    }
}

#[cfg(target_os = "linux")]
fn query_os() -> Option<StackBounds> {
    // `pthread_attr_t` is opaque; a 64-byte buffer is large
    // enough for all known glibc / musl implementations on
    // supported architectures (actual size is 56 bytes on glibc
    // x86_64, 56 on musl).
    #[repr(C, align(16))]
    struct PthreadAttr([u8; 64]);

    extern "C" {
        fn pthread_self() -> usize;
        fn pthread_getattr_np(thread: usize, attr: *mut PthreadAttr) -> i32;
        fn pthread_attr_destroy(attr: *mut PthreadAttr) -> i32;
        fn pthread_attr_getstack(
            attr: *const PthreadAttr,
            stackaddr: *mut *mut core::ffi::c_void,
            stacksize: *mut usize,
        ) -> i32;
    }

    let mut attr = PthreadAttr([0u8; 64]);
    let mut stackaddr: *mut core::ffi::c_void = core::ptr::null_mut();
    let mut stacksize: usize = 0;

    // SAFETY: `attr` is a writable buffer of pthread_attr_t size;
    // pthread_getattr_np initialises it. We pair the call with
    // pthread_attr_destroy on every exit path. pthread_self
    // returns the calling thread id, which is always valid.
    unsafe {
        let tid = pthread_self();
        if pthread_getattr_np(tid, &mut attr) != 0 {
            return None;
        }
        let rc = pthread_attr_getstack(&attr, &mut stackaddr, &mut stacksize);
        let _ = pthread_attr_destroy(&mut attr);
        if rc != 0 {
            return None;
        }
    }

    let low = stackaddr as usize;
    let high = low.checked_add(stacksize)?;
    if low == 0 || stacksize == 0 {
        None
    } else {
        Some(StackBounds { low, high })
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
fn query_os() -> Option<StackBounds> {
    extern "C" {
        fn pthread_self() -> usize;
        fn pthread_get_stackaddr_np(thread: usize) -> *mut core::ffi::c_void;
        fn pthread_get_stacksize_np(thread: usize) -> usize;
    }
    // SAFETY: pthread_self / pthread_get_*_np are documented to
    // accept the current thread id and return its stack
    // descriptor without side effects.
    let (addr_top, size) = unsafe {
        let tid = pthread_self();
        (
            pthread_get_stackaddr_np(tid) as usize,
            pthread_get_stacksize_np(tid),
        )
    };
    if addr_top == 0 || size == 0 {
        return None;
    }
    // On Darwin / BSD, `pthread_get_stackaddr_np` returns the
    // high (top) address of the stack region; subtract `size` to
    // get the low.
    let low = addr_top.checked_sub(size)?;
    Some(StackBounds {
        low,
        high: addr_top,
    })
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
)))]
fn query_os() -> Option<StackBounds> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_are_sane_on_host() {
        let b = current_stack_bounds().expect("host OS should be supported");
        assert!(b.low < b.high);
        // Stack should span at least 64 KB.
        assert!(b.high - b.low >= 64 * 1024);
        // NOTE: an earlier version of this test also asserted that
        // the address of a local variable lies inside [low, high).
        // That assertion does not hold under AddressSanitizer's
        // `detect_stack_use_after_return=1` mode, which heap-
        // allocates locals on a fake stack. The walker itself is
        // unaffected because it operates on the real FP chain.
    }

    #[test]
    fn cached_after_first_call() {
        let a = current_stack_bounds();
        let b = current_stack_bounds();
        assert_eq!(a.map(|x| (x.low, x.high)), b.map(|x| (x.low, x.high)));
    }
}
