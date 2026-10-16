//! What the integration tests share: a stand-in for the Codecov API, a
//! throwaway git repository, and a way to run the binary against both.

// Each test file compiles its own copy of this module and uses only part of it.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

pub(crate) const BIN: &str = env!("CARGO_BIN_EXE_codecov-coverage-cli");

/// Settings from the developer's shell that would leak into a run.
const INHERITED: &[&str] = &[
    "CODECOV_AUTH_TOKEN",
    "CODECOV_SERVICE",
    "CODECOV_OWNER",
    "CODECOV_REPO",
    "CODECOV_API_URL",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
    "GIT_DIR",
    "GIT_WORK_TREE",
];

#[derive(Debug, Clone)]
pub(crate) struct Request {
    /// As sent, still percent-encoded: `/api/v2/github/acme/repos/shop/file_report/src%2Fa.rs`.
    pub(crate) path: String,
    pub(crate) query: BTreeMap<String, String>,
    pub(crate) authorization: Option<String>,
}

/// A one-thread HTTP server that answers every request with `handler`.
pub(crate) struct Mock {
    pub(crate) url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Mock {
    pub(crate) fn start(handler: impl Fn(&Request) -> (u16, String) + Send + 'static) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                let (status, body) = handler(&request);
                recorded.lock().unwrap().push(request);
                let reason = match status {
                    200 => "OK",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    _ => "Internal Server Error",
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Mock { url, requests }
    }

    /// Answers every request with the same report.
    pub(crate) fn always(body: &str) -> Mock {
        let body = body.to_owned();
        Mock::start(move |_| (200, body.clone()))
    }

    pub(crate) fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let mut authorization = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("authorization")
        {
            authorization = Some(value.trim().to_owned());
        }
    }
    let target = request_line.split_whitespace().nth(1)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let query = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(key), decode(value))
        })
        .collect();
    Some(Request {
        path: path.to_owned(),
        query,
        authorization,
    })
}

pub(crate) fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap()
}

/// A directory under the system temp dir, removed again on drop. With
/// `git: true` it is a repository whose `origin` is `github.com/acme/shop`.
pub(crate) struct Dir {
    pub(crate) root: PathBuf,
}

impl Dir {
    pub(crate) fn new(name: &str, git: bool) -> Dir {
        let root = std::env::temp_dir().join(format!("codecov-coverage-cli-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let dir = Dir { root };
        if git {
            dir.git(&["init", "--quiet", "--initial-branch=main"]);
            dir.git(&["remote", "add", "origin", "git@github.com:acme/shop.git"]);
        }
        dir
    }

    pub(crate) fn write(&self, path: &str, content: &str) {
        let path = self.root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    /// Runs git with the developer's own configuration shut out.
    pub(crate) fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(&self.root)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// Stages everything and commits; returns the new commit's SHA.
    pub(crate) fn commit(&self, message: &str) -> String {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "--message", message]);
        self.git(&["rev-parse", "HEAD"])
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[derive(Debug)]
pub(crate) struct Run {
    pub(crate) code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl Run {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| panic!("{e}:\n{}", self.stdout))
    }
}

/// A command for the binary in `cwd`, pointed at `api` with a token set and
/// nothing inherited that could change the answer.
pub(crate) fn command(cwd: &Path, api: &str) -> Command {
    let mut command = Command::new(BIN);
    command.current_dir(cwd);
    for name in INHERITED {
        command.env_remove(name);
    }
    command
        .env("CODECOV_AUTH_TOKEN", "test-token")
        .env("CODECOV_API_URL", api)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

pub(crate) fn finish(command: &mut Command) -> Run {
    let output = command.output().unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

pub(crate) fn run(cwd: &Path, api: &str, args: &[&str]) -> Run {
    finish(command(cwd, api).args(args))
}
