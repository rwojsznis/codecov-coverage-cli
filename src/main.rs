//! The `codecov-coverage-cli` binary.
//!
//! Everything is under `cli`: the flags, the three modes and the printing
//! macros. This file is the entry point and the place a failed run's message
//! is printed.

// A panicking CLI is a bug: every mode returns `Result`. Declared here rather
// than in `Cargo.toml`, because a manifest `[lints]` table also covers
// `tests/`, where a panic *is* the failure report. `clippy.toml` exempts the
// unit tests in `src/`.
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod cli;

use crate::cli::out::errln;
use std::process::ExitCode;

fn main() -> ExitCode {
    match cli::run() {
        Ok(exit) => exit.into(),
        Err(failure) => {
            errln!("codecov-coverage-cli: {}", failure.message);
            failure.exit.into()
        }
    }
}
