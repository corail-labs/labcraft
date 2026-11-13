//! The worker of the web viewer, where its session runs (`docs/architecture.md`, 11.5), built for the browser by
//! `cargo xtask web`.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::string_slice
)]
#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
fn main() {
    labcraft_web::worker::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "labcraft-worker runs in a browser, as the worker of labcraft-web");
    std::process::ExitCode::from(2)
}
