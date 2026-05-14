//! Unix (Linux / macOS / *BSD) symbolicator using `addr2line` and
//! `object`.
//!
//! `Context` borrows from the parsed `object::File`, which borrows
//! from the raw binary bytes. We resolve the lifetime puzzle by
//! reading the binary once and leaking the bytes via `Box::leak`,
//! upgrading them to `'static`. The leaked allocation lives for
//! the rest of the process; for a profiler this is fine, since the
//! same process that opened the binary uses it for the duration.

use std::sync::OnceLock;

use addr2line::Context;
use object::read::File as ObjectFile;

use super::SymbolicatedFrame;

type Ctx = Context<addr2line::gimli::EndianRcSlice<addr2line::gimli::RunTimeEndian>>;

struct UnixSymbolicator {
    ctx: Ctx,
}

static SYMBOLICATOR: OnceLock<Option<UnixSymbolicator>> = OnceLock::new();

fn build() -> Option<UnixSymbolicator> {
    let path = super::self_binary::self_exe()?;
    let bytes = std::fs::read(path).ok()?;
    // Leak the bytes so the parsed `File` and the `Context` can
    // safely hold references for the rest of the process. Memory
    // cost: roughly the binary's on-disk size, one-time.
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    let obj = ObjectFile::parse(leaked).ok()?;
    let ctx = Context::new(&obj).ok()?;
    Some(UnixSymbolicator { ctx })
}

/// Resolve one address. Returns one or more frames; multiple
/// frames mean inlined call-site expansion at this address.
pub(crate) fn resolve(address: u64) -> Vec<SymbolicatedFrame> {
    let sym = SYMBOLICATOR.get_or_init(build);
    let Some(sym) = sym.as_ref() else {
        return vec![SymbolicatedFrame {
            address,
            function: None,
            file: None,
            line: None,
            inlined: false,
        }];
    };

    let mut out = Vec::new();
    let Ok(mut frames) = sym.ctx.find_frames(address).skip_all_loads() else {
        return vec![SymbolicatedFrame {
            address,
            function: None,
            file: None,
            line: None,
            inlined: false,
        }];
    };

    let mut idx = 0usize;
    while let Ok(Some(frame)) = frames.next() {
        let function = frame
            .function
            .as_ref()
            .and_then(|f| f.raw_name().ok())
            .map(|name| rustc_demangle::demangle(&name).to_string());

        let (file, line) = match frame.location {
            Some(loc) => (loc.file.map(std::path::PathBuf::from), loc.line),
            None => (None, None),
        };

        out.push(SymbolicatedFrame {
            address,
            function,
            file,
            line,
            inlined: idx > 0,
        });
        idx += 1;
    }

    if out.is_empty() {
        out.push(SymbolicatedFrame {
            address,
            function: None,
            file: None,
            line: None,
            inlined: false,
        });
    }

    out
}
