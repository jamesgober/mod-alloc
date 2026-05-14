//! Pure-Rust frame-pointer walk with hardening checks.
//!
//! Given an initial frame pointer (typically the result of
//! [`crate::backtrace::capture::current_fp`]) and the current
//! thread's stack bounds, walks up to 8 frames, returning the
//! captured return addresses.
//!
//! Safety strategy: every dereference is preceded by null,
//! alignment, in-range, and monotonicity checks (see
//! `.dev/DESIGN_v0.9.1.md` section 1). Any check failure stops
//! the walk and returns the frames captured so far. The walk is
//! also bounded to exactly 8 iterations as a final safety net.

use super::stack_bounds::StackBounds;

/// Maximum number of frames captured per allocation.
pub(crate) const MAX_FRAMES: usize = 8;

/// Frame buffer plus the count of valid entries.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Frames {
    pub frames: [u64; MAX_FRAMES],
    pub count: u8,
}

impl Frames {
    pub(crate) const fn empty() -> Self {
        Self {
            frames: [0; MAX_FRAMES],
            count: 0,
        }
    }
}

/// Walk the frame-pointer chain starting at `initial_fp`, using
/// `bounds` to gate each dereference. Returns up to `MAX_FRAMES`
/// return addresses.
///
/// The walk is "total": it terminates within `MAX_FRAMES`
/// iterations for any input, regardless of how the input bytes
/// look. This is the property exercised by the corruption-fuzz
/// tests.
#[inline]
pub(crate) fn walk(initial_fp: u64, bounds: StackBounds) -> Frames {
    let mut out = Frames::empty();
    let mut fp = initial_fp;
    let low = bounds.low as u64;
    let high = bounds.high as u64;

    let mut i = 0;
    while i < MAX_FRAMES {
        // Null check.
        if fp == 0 {
            break;
        }
        // Alignment check (16-byte alignment on SysV-x86_64 and AAPCS-aarch64).
        if fp & 0xF != 0 {
            break;
        }
        // Stack-range check: need at least 16 bytes readable starting at fp.
        if fp < low || fp.checked_add(16).map_or(true, |hi| hi > high) {
            break;
        }

        // Read saved FP and return address.
        // SAFETY: the four preceding checks establish that `fp`
        // points at 16 bytes of readable, 16-byte-aligned memory
        // inside the current thread's own stack region. Stack
        // pages are always mapped (the guard page sits at the
        // lowest page, which we excluded via `low`). The reads
        // produce a `u64` value treated as bits; we do not
        // dereference them further as pointers without the same
        // checks repeated on the next iteration.
        let new_fp = unsafe { core::ptr::read_volatile(fp as *const u64) };
        let return_addr = unsafe { core::ptr::read_volatile((fp + 8) as *const u64) };

        // Monotonicity: chain must progress upward (older frames
        // sit at higher addresses on stacks that grow down).
        if new_fp != 0 && new_fp <= fp {
            // Capture the return address we just read before
            // breaking; it is still valid.
            if return_addr != 0 {
                out.frames[i] = return_addr;
                out.count += 1;
            }
            break;
        }

        if return_addr == 0 {
            // Bottom of the chain.
            break;
        }

        out.frames[i] = return_addr;
        out.count += 1;
        fp = new_fp;
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_stack(words: &[u64]) -> Vec<u64> {
        // Caller arranges words as alternating [saved_fp, ret_addr, ...]
        // starting from the lowest address.
        words.to_vec()
    }

    fn bounds_for(buf: &[u64]) -> StackBounds {
        let base = buf.as_ptr() as usize;
        StackBounds {
            low: base,
            high: base + buf.len() * 8,
        }
    }

    #[test]
    fn empty_chain_returns_zero() {
        let buf: Vec<u64> = fake_stack(&[]);
        let b = StackBounds { low: 0, high: 0 };
        let frames = walk(0, b);
        assert_eq!(frames.count, 0);
        let _ = buf;
    }

    #[test]
    fn walks_two_frame_chain() {
        // Layout: at fp0 we have [fp1, ret0]; at fp1 we have [0, ret1].
        // We need addresses, so construct the chain in a Vec and
        // patch the saved-FP fields to point at the right offsets.
        let mut buf: Vec<u64> = vec![0; 16];
        let base = buf.as_mut_ptr() as u64;
        // fp0 at base + 0: saved_fp -> base + 16, ret0 = 0xAAAA
        buf[0] = base + 16;
        buf[1] = 0xAAAA;
        // fp1 at base + 16: saved_fp = 0 (end), ret1 = 0xBBBB
        buf[2] = 0;
        buf[3] = 0xBBBB;

        let b = bounds_for(&buf);
        let frames = walk(base, b);
        assert_eq!(frames.count, 2);
        assert_eq!(frames.frames[0], 0xAAAA);
        assert_eq!(frames.frames[1], 0xBBBB);
    }

    #[test]
    fn stops_at_null_saved_fp_with_zero_return_addr() {
        let mut buf: Vec<u64> = vec![0; 8];
        let base = buf.as_mut_ptr() as u64;
        buf[0] = 0; // saved_fp = 0
        buf[1] = 0; // ret = 0
        let b = bounds_for(&buf);
        let frames = walk(base, b);
        assert_eq!(frames.count, 0);
    }

    #[test]
    fn stops_at_misaligned_fp() {
        let mut buf: Vec<u64> = vec![0; 8];
        let base = buf.as_mut_ptr() as u64;
        let frames = walk(base + 1, bounds_for(&buf));
        assert_eq!(frames.count, 0);
    }

    #[test]
    fn stops_at_out_of_range_fp() {
        let buf: Vec<u64> = vec![0; 8];
        let bogus = (buf.as_ptr() as u64).wrapping_add(0x100000);
        let frames = walk(bogus, bounds_for(&buf));
        assert_eq!(frames.count, 0);
    }

    #[test]
    fn stops_at_non_monotonic_chain() {
        let mut buf: Vec<u64> = vec![0; 16];
        let base = buf.as_mut_ptr() as u64;
        // fp0 points "back" to itself, which is non-monotonic.
        buf[0] = base;
        buf[1] = 0xCAFE;
        let frames = walk(base, bounds_for(&buf));
        // First frame captured, then the next iteration sees
        // non-monotonic chain and stops.
        assert_eq!(frames.count, 1);
        assert_eq!(frames.frames[0], 0xCAFE);
    }

    #[test]
    fn caps_at_max_frames() {
        // Build a chain of 16 valid frames; expect walker to stop at 8.
        let mut buf: Vec<u64> = vec![0; 64];
        let base = buf.as_mut_ptr() as u64;
        for i in 0..15 {
            buf[i * 2] = base + ((i + 1) * 16) as u64;
            buf[i * 2 + 1] = 0x1000 + i as u64;
        }
        buf[30] = 0;
        buf[31] = 0;
        let frames = walk(base, bounds_for(&buf));
        assert_eq!(frames.count as usize, MAX_FRAMES);
    }
}
