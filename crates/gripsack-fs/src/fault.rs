//! Debug/test-only observation and fault injection at real filesystem calls.
//! Release builds inline directly to the operation; no environment lookups.

use std::{io, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    Write,
    Mode,
    FileSync,
    DirSync,
    FilePublish,
    TreePublish,
    Unlink,
    Mkdir,
    Symlink,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Before,
    After,
}

#[inline]
pub fn operation<T>(
    boundary: Boundary,
    path: &Path,
    perform: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    checkpoint(boundary, Edge::Before, path)?;
    let result = perform()?;
    checkpoint(boundary, Edge::After, path)?;
    Ok(result)
}

#[cfg(not(any(debug_assertions, test)))]
#[inline(always)]
fn checkpoint(_: Boundary, _: Edge, _: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(any(debug_assertions, test))]
fn checkpoint(boundary: Boundary, edge: Edge, path: &Path) -> io::Result<()> {
    use std::io::Write;
    use std::sync::{LazyLock, Mutex};
    struct Config {
        trace: Option<std::path::PathBuf>,
        cut: Option<usize>,
        abort: bool,
        ordinal: usize,
    }
    static CONFIG: LazyLock<Mutex<Config>> = LazyLock::new(|| {
        Mutex::new(Config {
            trace: std::env::var_os("GRIPSACK_FS_TRACE").map(Into::into),
            cut: std::env::var("GRIPSACK_FS_CUT")
                .ok()
                .and_then(|s| s.parse().ok()),
            abort: std::env::var("GRIPSACK_FS_FAULT").as_deref() == Ok("abort"),
            ordinal: 0,
        })
    });
    #[cfg(test)]
    RECORD.with(|record| {
        if let Some(events) = &mut *record.borrow_mut() {
            events.push(Event {
                boundary,
                edge,
                path: path.to_owned(),
            });
        }
    });
    let mut state = CONFIG.lock().expect("fault observer");
    if state.trace.is_none() && state.cut.is_none() {
        return Ok(());
    }
    state.ordinal += 1;
    if let Some(trace) = &state.trace {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(trace)?;
        // Debug path formatting escapes newlines/tabs: each event is one row.
        writeln!(file, "{}\t{edge:?}\t{boundary:?}\t{path:?}", state.ordinal)?;
    }
    if state.cut == Some(state.ordinal) {
        if state.abort {
            std::process::abort();
        }
        return Err(io::Error::other(format!("injected {edge:?} {boundary:?}")));
    }
    Ok(())
}

#[inline]
pub(crate) fn force_copy() -> bool {
    #[cfg(test)]
    if FORCE_COPY.with(|force| force.get()) {
        return true;
    }
    #[cfg(debug_assertions)]
    {
        std::env::var_os("GRIPSACK_FS_FORCE_COPY").is_some()
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct Event {
    pub boundary: Boundary,
    pub edge: Edge,
    pub path: std::path::PathBuf,
}
#[cfg(test)]
thread_local! {
    static RECORD: std::cell::RefCell<Option<Vec<Event>>> = const { std::cell::RefCell::new(None) };
    static FORCE_COPY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
pub(crate) fn capture<T>(force_copy: bool, perform: impl FnOnce() -> T) -> (T, Vec<Event>) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            RECORD.with(|r| {
                r.borrow_mut().take();
            });
            FORCE_COPY.with(|f| f.set(false));
        }
    }
    RECORD.with(|r| *r.borrow_mut() = Some(Vec::new()));
    FORCE_COPY.with(|f| f.set(force_copy));
    let reset = Reset;
    let result = perform();
    let events = RECORD.with(|r| r.borrow_mut().take().expect("capturing"));
    drop(reset);
    (result, events)
}
