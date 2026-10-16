//! Codecov line coverage for the command line: per file, for a whole
//! repository, or for only the lines a branch changed.
//!
//! The library has no I/O of its own except in `api` (HTTP) and `git` (the
//! `git` binary). Everything else is a pure function over their output, so it
//! can be tested without a network or a repository.

// A panicking library is a bug: this crate returns `Result` everywhere.
// Declared here rather than in `Cargo.toml`, because a manifest `[lints]` table
// also covers `tests/`, where a panic *is* the failure report. `clippy.toml`
// exempts the unit tests in `src/`.
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod api;
pub mod diff;
pub mod git;
pub mod paths;
pub mod report;
