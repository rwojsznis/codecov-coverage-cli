//! A reader that quits early (`| head`) closes the pipe.
//!
//! Rust ignores `SIGPIPE`, so the failed write comes back as an error and
//! `println!` panics on it. That is not a tool failure: the output simply has
//! nowhere left to go.

mod common;

use common::{Dir, Mock, command};
use std::io::Read;
use std::process::Stdio;

#[test]
fn a_closed_stdout_is_not_a_panic() {
    let dir = Dir::new("pipe", true);
    // Enough files that the output is far larger than a pipe buffer.
    let files: Vec<String> = (0..5000)
        .map(|i| {
            format!(
                r#"{{"name":"src/file_{i}.rs","totals":{{"lines":4,"hits":1,"misses":3}},"line_coverage":[[1,1],[2,1],[4,1],[5,0]]}}"#
            )
        })
        .collect();
    let mock = Mock::always(&format!(r#"{{"files":[{}]}}"#, files.join(",")));

    for args in [
        vec!["--all"],
        vec!["--all", "--json"],
        vec!["--all", "--raw"],
    ] {
        let mut child = command(&dir.root, &mock.url)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        let status = child.wait().unwrap();
        assert!(
            !stderr.contains("panicked"),
            "{args:?} panicked on a closed pipe:\n{stderr}"
        );
        assert_eq!(status.code(), Some(0), "{args:?}: {stderr}");
    }
}
