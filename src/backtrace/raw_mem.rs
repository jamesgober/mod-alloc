//! Platform-specific raw page allocation, used for the global
//! aggregation table.
//!
//! The backtrace path cannot recursively call `ModAlloc::alloc`
//! when initialising its own state, so this module bypasses the
//! Rust allocator entirely and goes to the kernel through
//! `VirtualAlloc` on Windows and `mmap` on POSIX. The reentrancy
//! guard from `v0.9.0` protects callers in case any libc internal
//! does touch the heap during these syscalls.

use core::ffi::c_void;

/// Allocate `size` bytes of zero-initialised, read-write,
/// anonymous memory. Returns null on failure.
///
/// On POSIX, uses `mmap(NULL, size, PROT_READ|PROT_WRITE,
/// MAP_PRIVATE|MAP_ANONYMOUS, -1, 0)`. On Windows, uses
/// `VirtualAlloc(NULL, size, MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE)`.
///
/// # Safety
///
/// The returned pointer (if non-null) must be released with
/// [`free_pages`] using the same `size`.
pub(crate) unsafe fn alloc_pages(size: usize) -> *mut u8 {
    #[cfg(target_os = "windows")]
    {
        extern "system" {
            fn VirtualAlloc(
                lp_address: *mut c_void,
                dw_size: usize,
                fl_allocation_type: u32,
                fl_protect: u32,
            ) -> *mut c_void;
        }
        const MEM_COMMIT: u32 = 0x1000;
        const MEM_RESERVE: u32 = 0x2000;
        const PAGE_READWRITE: u32 = 0x04;
        // SAFETY: VirtualAlloc with null base address requests
        // the system pick the location. Memory is zero-init by
        // contract.
        VirtualAlloc(
            core::ptr::null_mut(),
            size,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        ) as *mut u8
    }

    #[cfg(not(target_os = "windows"))]
    {
        extern "C" {
            fn mmap(
                addr: *mut c_void,
                len: usize,
                prot: i32,
                flags: i32,
                fd: i32,
                offset: i64,
            ) -> *mut c_void;
        }
        const PROT_READ: i32 = 1;
        const PROT_WRITE: i32 = 2;
        const MAP_PRIVATE: i32 = 2;
        #[cfg(target_os = "linux")]
        const MAP_ANONYMOUS: i32 = 0x20;
        #[cfg(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        const MAP_ANONYMOUS: i32 = 0x1000;
        #[cfg(not(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        )))]
        const MAP_ANONYMOUS: i32 = 0x20;

        // SAFETY: mmap with null address requests the kernel
        // pick the location. Anonymous mapping is zero-init by
        // POSIX contract.
        let ptr = mmap(
            core::ptr::null_mut(),
            size,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        );
        // mmap returns MAP_FAILED ((void*)-1) on failure.
        if ptr as isize == -1 {
            core::ptr::null_mut()
        } else {
            ptr as *mut u8
        }
    }
}

/// Release pages previously returned by [`alloc_pages`].
///
/// # Safety
///
/// `ptr` must be non-null and was returned by `alloc_pages(size)`
/// with the same `size`. Caller must guarantee no other thread
/// holds an active reference into the region.
pub(crate) unsafe fn free_pages(ptr: *mut u8, size: usize) {
    if ptr.is_null() {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        extern "system" {
            fn VirtualFree(lp_address: *mut c_void, dw_size: usize, dw_free_type: u32) -> i32;
        }
        const MEM_RELEASE: u32 = 0x8000;
        let _ = size; // unused on Windows
                      // SAFETY: ptr was returned by VirtualAlloc per caller's
                      // contract. MEM_RELEASE requires size == 0.
        let _ = VirtualFree(ptr as *mut c_void, 0, MEM_RELEASE);
    }
    #[cfg(not(target_os = "windows"))]
    {
        extern "C" {
            fn munmap(addr: *mut c_void, len: usize) -> i32;
        }
        // SAFETY: ptr/size pair was returned by mmap per caller's
        // contract.
        let _ = munmap(ptr as *mut c_void, size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_and_free_round_trip() {
        // SAFETY: standard allocate/use/free contract.
        unsafe {
            let p = alloc_pages(4096);
            assert!(!p.is_null());
            // Region is zero-init.
            for i in 0..4096 {
                assert_eq!(*p.add(i), 0);
            }
            // Writable.
            *p = 0xAA;
            assert_eq!(*p, 0xAA);
            free_pages(p, 4096);
        }
    }

    #[test]
    fn alloc_zero_safely() {
        // SAFETY: 4 KB is a reasonable allocation; tests release path.
        unsafe {
            let p = alloc_pages(64 * 1024);
            assert!(!p.is_null());
            free_pages(p, 64 * 1024);
        }
    }
}
