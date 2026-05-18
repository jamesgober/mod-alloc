//! DHAT-compatible JSON output for the per-call-site report.
//!
//! Behind the `dhat-compat` cargo feature (which already implies
//! `backtraces`). With `symbolicate` additionally enabled, frame
//! strings carry function names and source locations; without,
//! they carry raw hex addresses.
//!
//! ## Schema
//!
//! See `.dev/DESIGN_v0.9.3.md` for the locked field-by-field
//! schema. Summary:
//!
//! - `dhatFileVersion: 2`, `mode: "rust-heap"`, `verb: "Allocated"`.
//! - `bklt: false`, `bkacc: false` — we do not track allocation
//!   lifetimes or per-block accesses.
//! - `pps[]` — one entry per surviving call site with `tb`
//!   (total bytes), `tbk` (count), `eb`/`ebk` (zero), and `fs`
//!   (frame-table indices, top of stack first).
//! - `ftbl[]` — deduplicated frame strings; index 0 is `[root]`.
//!
//! ## No new deps
//!
//! The JSON writer is hand-rolled. The frame table is a plain
//! `HashMap<String, u32>`. The schema is small and fixed; pulling
//! in `serde` / `serde_json` only to emit ~200 lines of known
//! shape would undermine the zero-runtime-dep stance.

use std::fmt::Write as _;
use std::io;
use std::path::Path;

mod frames;
mod writer;

use frames::{build, FrameTable, ProgramPoint};
use writer::{write_json_string, write_key};

/// Render the current report as a DHAT-compatible JSON string.
///
/// Allocates. Safe to call from non-allocator contexts only
/// (ordinary user code outside the global-allocator hook). With
/// the `symbolicate` feature, this internally drives
/// symbolication and returns resolved frame strings.
pub(crate) fn dhat_json_string() -> String {
    let (ftbl, pps) = build();
    render(&ftbl, &pps)
}

/// Render the report and write it to `path`.
pub(crate) fn write_dhat_json(path: &Path) -> io::Result<()> {
    let bytes = dhat_json_string();
    std::fs::write(path, bytes)
}

fn render(ftbl: &FrameTable, pps: &[ProgramPoint]) -> String {
    let mut out = String::with_capacity(256 + pps.len() * 64 + ftbl.entries.len() * 32);

    out.push('{');

    // Header.
    write_key(&mut out, "dhatFileVersion");
    out.push_str("2,");

    write_key(&mut out, "mode");
    write_json_string(&mut out, "rust-heap");
    out.push(',');

    write_key(&mut out, "verb");
    write_json_string(&mut out, "Allocated");
    out.push(',');

    write_key(&mut out, "bklt");
    out.push_str("false,");

    write_key(&mut out, "bkacc");
    out.push_str("false,");

    write_key(&mut out, "bu");
    write_json_string(&mut out, "byte");
    out.push(',');

    write_key(&mut out, "bsu");
    write_json_string(&mut out, "bytes");
    out.push(',');

    write_key(&mut out, "bksu");
    write_json_string(&mut out, "blocks");
    out.push(',');

    write_key(&mut out, "tu");
    write_json_string(&mut out, "instrs");
    out.push(',');

    write_key(&mut out, "Mtu");
    write_json_string(&mut out, "Minstr");
    out.push(',');

    write_key(&mut out, "tuth");
    out.push_str("0,");

    write_key(&mut out, "cmd");
    write_json_string(&mut out, &current_cmd());
    out.push(',');

    write_key(&mut out, "pid");
    let _ = write!(out, "{}", std::process::id());
    out.push(',');

    write_key(&mut out, "tg");
    out.push_str("0,");

    write_key(&mut out, "te");
    out.push_str("0,");

    // pps.
    write_key(&mut out, "pps");
    out.push('[');
    for (i, pp) in pps.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_pp(&mut out, pp);
    }
    out.push(']');
    out.push(',');

    // ftbl.
    write_key(&mut out, "ftbl");
    out.push('[');
    for (i, frame) in ftbl.entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_json_string(&mut out, frame);
    }
    out.push(']');

    out.push('}');
    out
}

fn write_pp(out: &mut String, pp: &ProgramPoint) {
    out.push('{');

    write_key(out, "tb");
    let _ = write!(out, "{},", pp.total_bytes);

    write_key(out, "tbk");
    let _ = write!(out, "{},", pp.count);

    write_key(out, "eb");
    out.push_str("0,");

    write_key(out, "ebk");
    out.push_str("0,");

    write_key(out, "fs");
    out.push('[');
    for (i, idx) in pp.frame_indices.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{idx}");
    }
    out.push(']');

    out.push('}');
}

fn current_cmd() -> String {
    let mut parts = std::env::args_os();
    let Some(first) = parts.next() else {
        return "<unknown>".to_string();
    };
    let mut out = first.to_string_lossy().into_owned();
    for arg in parts {
        out.push(' ');
        out.push_str(&arg.to_string_lossy());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_report_is_wellformed() {
        // We can't easily reset the global table from here (it
        // sits behind `_reset_for_test` on a different module),
        // so instead just verify the structural shape of an
        // emitted document. The full empty-report case is
        // covered in the integration test where the test binary
        // can install the table fresh.
        let s = dhat_json_string();
        assert!(s.starts_with('{'));
        assert!(s.ends_with('}'));
        for key in [
            "\"dhatFileVersion\":2",
            "\"mode\":\"rust-heap\"",
            "\"verb\":\"Allocated\"",
            "\"bklt\":false",
            "\"bkacc\":false",
            "\"pps\":[",
            "\"ftbl\":[",
            "\"[root]\"",
        ] {
            assert!(s.contains(key), "missing key/value: {key}\nfull: {s}");
        }
    }

    #[test]
    fn current_cmd_returns_nonempty() {
        let s = current_cmd();
        assert!(!s.is_empty());
    }
}
