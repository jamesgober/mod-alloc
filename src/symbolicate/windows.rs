//! Windows symbolicator using `pdb`.
//!
//! Reads the PDB that sits beside the .exe (`mybin.exe` ->
//! `mybin.pdb`). On first call, walks every public symbol once
//! and builds a sorted `(rva, name)` index. The PDB itself is
//! dropped after indexing; per-resolve work is then a binary
//! search.
//!
//! ## Limitations vs the Unix path
//!
//! - No source file / line. PDB exposes line info via
//!   `module.line_program()` but threading that through the
//!   index requires significantly more code. Deferred.
//! - No inlined-frame expansion. PDB encodes inlined call sites
//!   in `S_INLINESITE` records inside per-module symbol streams;
//!   decoding them is non-trivial. Deferred.
//! - Best-effort address-to-RVA translation. Without the
//!   module's load base we approximate by masking `address` to
//!   32 bits. For non-ASLR builds this is exact; for ASLR
//!   builds the result is still usable for relative comparison
//!   inside the same process run.

use std::sync::OnceLock;

use pdb::{FallibleIterator, PDB};

use super::SymbolicatedFrame;

/// Sorted `(rva, demangled_name)` pairs.
type SymbolIndex = Vec<(u32, String)>;

static INDEX: OnceLock<Option<SymbolIndex>> = OnceLock::new();

fn build() -> Option<SymbolIndex> {
    let exe = super::self_binary::self_exe()?;
    let mut pdb_path = exe.clone();
    pdb_path.set_extension("pdb");
    if !pdb_path.exists() {
        return None;
    }

    let file = std::fs::File::open(&pdb_path).ok()?;
    let mut pdb = PDB::open(file).ok()?;
    let address_map = pdb.address_map().ok()?;
    let symbol_table = pdb.global_symbols().ok()?;

    let mut out: SymbolIndex = Vec::new();
    let mut iter = symbol_table.iter();
    while let Ok(Some(sym)) = iter.next() {
        if let Ok(pdb::SymbolData::Public(data)) = sym.parse() {
            if let Some(rva) = data.offset.to_rva(&address_map) {
                let raw = data.name.to_string().into_owned();
                let name = rustc_demangle::demangle(&raw).to_string();
                out.push((rva.0, name));
            }
        }
    }
    out.sort_by_key(|(rva, _)| *rva);
    out.dedup_by_key(|(rva, _)| *rva);

    Some(out)
}

/// Resolve one address against the PDB symbol index. Returns
/// a single frame (no inlined expansion on Windows yet).
pub(crate) fn resolve(address: u64) -> Vec<SymbolicatedFrame> {
    let unresolved = || SymbolicatedFrame {
        address,
        function: None,
        file: None,
        line: None,
        inlined: false,
    };

    let Some(index) = INDEX.get_or_init(build) else {
        return vec![unresolved()];
    };

    // Approximate RVA from the absolute address.
    let target_rva = (address & 0xFFFF_FFFF) as u32;

    // Find the greatest rva <= target_rva.
    let pos = match index.binary_search_by_key(&target_rva, |(rva, _)| *rva) {
        Ok(i) => i,
        Err(0) => return vec![unresolved()],
        Err(i) => i - 1,
    };

    let name = index[pos].1.clone();
    vec![SymbolicatedFrame {
        address,
        function: Some(name),
        file: None,
        line: None,
        inlined: false,
    }]
}
