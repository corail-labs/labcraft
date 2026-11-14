//! What a panic said, for the report that names it: `LC0099` on a file, `internal` for a command. The hook that keeps
//! it is set once by the app, before any work.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;

use serde_json::Value;

use crate::bound::{MAX_RESULT, fit};
use crate::error::CommandError;

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

/// The last resort of a command (D-18; `docs/commands.md`, section 3): its work, its result bounded as any result is,
/// and a panic of it caught. `Err` is that panic, already the `internal` error to give; the caller may first forget what
/// the command was changing.
pub(crate) fn guarded(id: &str, work: impl FnOnce() -> Result<Value, CommandError>) -> Result<Result<Value, CommandError>, CommandError> {
    panic::catch_unwind(AssertUnwindSafe(work))
        .map(|result| result.map(|value| fit(value, MAX_RESULT)))
        .map_err(|_| CommandError::internal(format!("a bug of LabCraft in `{id}`, caught: {}", last_panic())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_of_a_command_becomes_internal_and_its_result_is_bounded() {
        let caught = guarded("file.read", || panic!("half done")).unwrap_err();
        assert_eq!(caught.code, "internal");
        assert!(caught.message.starts_with("a bug of LabCraft in `file.read`, caught: "), "{}", caught.message);
        let refused = guarded("file.read", || Err(CommandError::new("path.invalid", "no"))).unwrap();
        assert_eq!(refused.unwrap_err().code, "path.invalid");
        let long = Value::String("x".repeat(MAX_RESULT + 1));
        assert_eq!(guarded("file.read", || Ok(long.clone())).unwrap().unwrap(), fit(long, MAX_RESULT));
    }
}
