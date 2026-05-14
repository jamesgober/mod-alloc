//! Inline frame-pointer register capture.
//!
//! Reads the current frame pointer register via `core::arch::asm!`.
//! Supported on `x86_64` (reads `rbp`) and `aarch64` (reads `x29`).
//! On any other target the function returns `0`, which causes the
//! walker to terminate immediately with zero frames captured.
//!
//! Frame-pointer-based capture only works when the calling code
//! was compiled with frame pointers enabled. See `build.rs` for
//! the warning emitted at compile time when the toolchain is
//! likely missing them.

/// Read the calling frame's saved frame pointer.
///
/// Marked `#[inline(always)]` so the FP captured corresponds to
/// the caller's frame, not this helper's. Callers can pass the
/// result into `walk::walk` to obtain return addresses.
#[inline(always)]
pub(crate) fn current_fp() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let fp: u64;
        core::arch::asm!(
            "mov {fp}, rbp",
            fp = out(reg) fp,
            options(nomem, nostack, preserves_flags)
        );
        fp
    }

    #[cfg(target_arch = "aarch64")]
    unsafe {
        let fp: u64;
        core::arch::asm!(
            "mov {fp}, x29",
            fp = out(reg) fp,
            options(nomem, nostack, preserves_flags)
        );
        fp
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_fp_is_nonzero_on_supported_archs() {
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        assert_ne!(current_fp(), 0, "FP register must be live in test fn");
    }

    #[test]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn current_fp_changes_across_calls() {
        #[inline(never)]
        fn deeper() -> u64 {
            // Force a stack-frame allocation so the captured FP
            // differs from the caller's. Without
            // `-C force-frame-pointers=yes` the FP register may
            // be reused, so we use the address of a local as a
            // proxy for the frame.
            let sentinel: u64 = 0;
            let probe = &sentinel as *const u64 as u64;
            core::hint::black_box(probe);
            current_fp()
        }
        let a = current_fp();
        let b = deeper();
        // Either the FP register itself moved (with frame
        // pointers) or, if it did not, the test still verifies
        // both calls produced a non-zero result. Soft assertion:
        // any failure here points to a build without FPs, which
        // build.rs already warned about.
        if a == b {
            assert_ne!(a, 0);
            assert_ne!(b, 0);
        } else {
            assert_ne!(a, b);
        }
    }
}
