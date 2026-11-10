//! What a panic said, for the report that names it: `LC0099` on a file, `internal` for a command. The hook that keeps
//! it is set once by the app, before any work.

use std::panic;
use std::sync::Mutex;

/// What the panic hook saw last.
static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

/// A panic is reported, not printed: the stderr of the command line is for wrong usage alone.
pub fn keep_panics_for_the_report() {
    panic::set_hook(Box::new(|info| {
        if let Ok(mut last) = LAST_PANIC.lock() {
            *last = Some(info.to_string());
        }
    }));
}

/// As [`keep_panics_for_the_report`], but the hook that was there still runs: the desktop viewer prints a panic of
/// its window, as any program does, and keeps the message of one caught in its session for its `LC0099`.
pub fn keep_panics_and_print_them() {
    let before = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if let Ok(mut last) = LAST_PANIC.lock() {
            *last = Some(info.to_string());
        }
        before(info);
    }));
}

/// The last panic seen, once: the next call says only "a panic" until another one comes.
pub fn last_panic() -> String {
    LAST_PANIC.lock().ok().and_then(|mut last| last.take()).unwrap_or_else(|| "a panic".to_owned())
}
