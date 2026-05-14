//! Unix (Linux / macOS / *BSD) symbolicator using `addr2line` and
//! `object`.
//!
//! `addr2line::Context<EndianRcSlice<_>>` (the type returned by
//! the default `Context::new` constructor) contains `Rc<[u8]>` in
//! its readers plus internal `UnsafeCell`-backed memoisation, so
//! it is fundamentally `!Send + !Sync`. We sidestep the cross-
//! thread sharing requirement by holding the symbolicator in a
//! `thread_local!` cell. Each thread that calls
//! `symbolicated_report()` builds its own context on first use;
//! the per-address cache in `super::report` then deduplicates the
//! per-address resolution work across threads.
//!
//! Memory cost: roughly the binary's on-disk size per thread that
//! ever symbolicated. Bytes are leaked once per thread via
//! `Box::leak` so the context's `'static` references remain valid
//! for the rest of the process.

use std::cell::RefCell;

use addr2line::Context;
use object::read::File as ObjectFile;

use super::SymbolicatedFrame;

type Ctx = Context<addr2line::gimli::EndianRcSlice<addr2line::gimli::RunTimeEndian>>;

struct UnixSymbolicator {
    ctx: Ctx,
}

thread_local! {
    // Outer `Option` distinguishes "not yet tried" (None) from
    // "tried and failed" (Some(None)). After the first call, the
    // outer is always Some, so we never retry the failed-build
    // path on subsequent calls.
    static SYMBOLICATOR: RefCell<Option<Option<UnixSymbolicator>>> =
        const { RefCell::new(None) };
}

fn build() -> Option<UnixSymbolicator> {
    let path = super::self_binary::self_exe()?;
    let bytes = std::fs::read(path).ok()?;
    // Leak the bytes so the parsed `File` and the `Context` can
    // safely hold references for the rest of the process. Memory
    // cost: roughly the binary's on-disk size, one-time per
    // thread that ever symbolicates.
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    let obj = ObjectFile::parse(leaked).ok()?;
    let ctx = Context::new(&obj).ok()?;
    Some(UnixSymbolicator { ctx })
}

/// Resolve one address. Returns one or more frames; multiple
/// frames mean inlined call-site expansion at this address.
pub(crate) fn resolve(address: u64) -> Vec<SymbolicatedFrame> {
    let unresolved = || {
        vec![SymbolicatedFrame {
            address,
            function: None,
            file: None,
            line: None,
            inlined: false,
        }]
    };

    SYMBOLICATOR.with(|cell| {
        let mut guard = match cell.try_borrow_mut() {
            Ok(g) => g,
            Err(_) => return unresolved(),
        };
        let sym = guard.get_or_insert_with(build);
        let Some(sym) = sym.as_ref() else {
            return unresolved();
        };

        let mut out = Vec::new();
        let Ok(mut frames) = sym.ctx.find_frames(address).skip_all_loads() else {
            return unresolved();
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
    })
}
