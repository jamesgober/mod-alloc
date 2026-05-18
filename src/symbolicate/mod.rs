//! Symbolication for the per-call-site report (v0.9.2).
//!
//! Behind the `symbolicate` cargo feature. Turns the raw return
//! addresses captured by v0.9.1's backtrace path into
//! `(function, file, line)` tuples at *report-generation time*.
//! Never invoked from the alloc hot path.
//!
//! ## Dependencies
//!
//! See `.dev/DIRECTIVES.md` section 2.2 for the approved external-
//! dep exception. Default builds, the `counters` feature, and
//! the `backtraces` feature remain zero-runtime-dep. The
//! `symbolicate` feature pulls in:
//!
//! - `addr2line` + `object` + `rustc-demangle` (all targets)
//! - `pdb` (Windows only)
//!
//! All are pure-Rust, MSRV 1.75-compatible.
//!
//! ## Platform parity
//!
//! Linux / macOS produce richer output than Windows (DWARF
//! inlining info is more complete than PDB's `S_INLINESITE`).
//! This is accepted; the asymmetry is documented and not gated.

use std::path::PathBuf;

pub(crate) mod self_binary;

#[cfg(unix)]
pub(crate) mod unix;

#[cfg(windows)]
pub(crate) mod windows;

pub(crate) mod report;

/// One symbolicated frame within a call site.
///
/// `function`, `file`, and `line` are each `None` when the
/// underlying debug info did not resolve to a name / location
/// (stripped binaries, FFI frames into system libraries, etc.).
/// The raw `address` is always preserved.
///
/// # Stability
///
/// Marked `#[non_exhaustive]` as of v1.0.0. Future minor
/// versions may add fields (e.g. column number, symbol kind,
/// crate-of-origin). Read fields by name; iterate via
/// [`ModAlloc::symbolicated_report`](crate::ModAlloc::symbolicated_report).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SymbolicatedFrame {
    /// Original raw return address from the capture path.
    pub address: u64,
    /// Demangled Rust function name, if resolvable.
    pub function: Option<String>,
    /// Source file path, if resolvable.
    pub file: Option<PathBuf>,
    /// Source line number, if resolvable.
    pub line: Option<u32>,
    /// True if this frame represents an inlined call site that
    /// expanded from the same return address as a previous frame
    /// in the parent `SymbolicatedCallSite.frames` vector.
    pub inlined: bool,
}

/// One symbolicated call site (counterpart of
/// [`CallSiteStats`](crate::CallSiteStats)).
///
/// Contains the aggregated counters from the raw report plus a
/// vector of resolved frames in top-of-stack-first order.
///
/// # Stability
///
/// Marked `#[non_exhaustive]` as of v1.0.0. Future minor
/// versions may add fields without bumping the major version.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SymbolicatedCallSite {
    /// Allocations attributed to this site.
    pub count: u64,
    /// Total bytes allocated at this site.
    pub total_bytes: u64,
    /// Resolved frames, top of stack first. May contain frames
    /// with `inlined = true` representing inlined call sites
    /// expanded from a single physical return address.
    pub frames: Vec<SymbolicatedFrame>,
}

pub use report::symbolicated_report;
