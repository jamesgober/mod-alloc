//! Frame-table construction.
//!
//! Walks the per-call-site report and builds two parallel
//! structures consumed by the JSON emitter:
//!
//! 1. `ftbl: Vec<String>` — deduplicated frame descriptions in
//!    the format `"0x{addr:x}: <name> (<file>:<line>)"`. Index 0
//!    is always the literal string `"[root]"`.
//! 2. `pps: Vec<ProgramPoint>` — one entry per call site,
//!    carrying counters and a `Vec<u32>` of frame-table indices
//!    in top-of-stack-first order.
//!
//! The implementation cfg-splits on the `symbolicate` feature:
//! when active, frames are rendered from the symbolicated
//! report (function name + file/line where available); when not,
//! frames are raw hex addresses with `<unresolved>` placeholders.

use std::collections::HashMap;

pub(super) struct ProgramPoint {
    pub(super) total_bytes: u64,
    pub(super) count: u64,
    pub(super) frame_indices: Vec<u32>,
}

pub(super) struct FrameTable {
    pub(super) entries: Vec<String>,
    intern: HashMap<String, u32>,
}

impl FrameTable {
    fn new() -> Self {
        let mut entries = Vec::with_capacity(8);
        entries.push("[root]".to_string());
        Self {
            entries,
            intern: HashMap::new(),
        }
    }

    fn intern(&mut self, frame: String) -> u32 {
        if let Some(&idx) = self.intern.get(&frame) {
            return idx;
        }
        let idx = self.entries.len() as u32;
        self.entries.push(frame.clone());
        self.intern.insert(frame, idx);
        idx
    }
}

/// Build the `(ftbl, pps)` pair for the current process.
///
/// With the `symbolicate` feature, this also drains the
/// per-call-site table (via `symbolicated_report`) so that the
/// caller's `dhat_json_string` is a one-stop call. Without
/// `symbolicate`, we call `call_sites_report` directly.
pub(super) fn build() -> (FrameTable, Vec<ProgramPoint>) {
    let mut ftbl = FrameTable::new();
    let mut pps = Vec::new();

    #[cfg(feature = "symbolicate")]
    {
        let report = crate::symbolicate::symbolicated_report();
        for site in report {
            let mut indices = Vec::with_capacity(site.frames.len());
            for frame in &site.frames {
                let s = format_symbolicated_frame(frame);
                indices.push(ftbl.intern(s));
            }
            pps.push(ProgramPoint {
                total_bytes: site.total_bytes,
                count: site.count,
                frame_indices: indices,
            });
        }
    }

    #[cfg(not(feature = "symbolicate"))]
    {
        let report = crate::backtrace::call_sites_report();
        for site in report {
            let n = site.frame_count as usize;
            let mut indices = Vec::with_capacity(n);
            for i in 0..n {
                let s = format_raw_frame(site.frames[i]);
                indices.push(ftbl.intern(s));
            }
            pps.push(ProgramPoint {
                total_bytes: site.total_bytes,
                count: site.count,
                frame_indices: indices,
            });
        }
    }

    (ftbl, pps)
}

#[cfg(not(feature = "symbolicate"))]
fn format_raw_frame(addr: u64) -> String {
    format!("0x{addr:x}: <unresolved>")
}

#[cfg(feature = "symbolicate")]
fn format_symbolicated_frame(frame: &crate::SymbolicatedFrame) -> String {
    use std::fmt::Write as _;

    let mut s = String::with_capacity(64);
    let _ = write!(s, "0x{:x}: ", frame.address);

    match frame.function.as_deref() {
        Some(name) => s.push_str(name),
        None => s.push_str("<unresolved fn>"),
    }

    if frame.inlined {
        s.push_str(" [inlined]");
    }

    if let Some(file) = frame.file.as_ref() {
        s.push_str(" (");
        s.push_str(&file.display().to_string());
        s.push(':');
        match frame.line {
            Some(line) => {
                let _ = write!(s, "{line}");
            }
            None => s.push('?'),
        }
        s.push(')');
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_always_index_zero() {
        let ft = FrameTable::new();
        assert_eq!(ft.entries[0], "[root]");
    }

    #[test]
    fn intern_deduplicates() {
        let mut ft = FrameTable::new();
        let a = ft.intern("0x1: foo".to_string());
        let b = ft.intern("0x2: bar".to_string());
        let a2 = ft.intern("0x1: foo".to_string());
        assert_eq!(a, 1);
        assert_eq!(b, 2);
        assert_eq!(a, a2);
        assert_eq!(ft.entries.len(), 3);
    }

    #[cfg(not(feature = "symbolicate"))]
    #[test]
    fn raw_frame_format() {
        assert_eq!(format_raw_frame(0xdeadbeef), "0xdeadbeef: <unresolved>");
    }
}
