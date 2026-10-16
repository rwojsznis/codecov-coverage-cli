//! `--all`: every file of one commit with missed or partial lines.

mod common;

use common::{Dir, Mock, Request, run};
use serde_json::json;

/// Four files, three of them with gaps. Honors the `path` prefix filter the
/// way Codecov does.
fn codecov() -> Mock {
    let files = json!([
        {"name": "src/index.ts", "totals": {"lines": 10, "hits": 5, "misses": 3, "partials": 2, "coverage": 50.0},
         "line_coverage": [[1,0],[3,1],[4,1],[5,1],[7,2],[9,1],[10,2]]},
        {"name": "src/codecov.ts", "totals": {"lines": 10, "hits": 10, "misses": 0, "partials": 0, "coverage": 100.0},
         "line_coverage": [[1,0]]},
        {"name": "src/cli.ts", "totals": {"lines": 20, "hits": 12, "misses": 8, "partials": 0, "coverage": 60.0},
         "line_coverage": [[2,1],[3,1],[4,1],[5,1],[6,1],[7,1],[8,1],[20,1]]},
        {"name": "src2/other.ts", "totals": {"lines": 4, "hits": 3, "misses": 0, "partials": 1, "coverage": 75.0},
         "line_coverage": [[4,2]]},
    ]);
    Mock::start(move |request: &Request| {
        let prefix = request.query.get("path").cloned().unwrap_or_default();
        let matching: Vec<_> = files
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["name"].as_str().unwrap().starts_with(&prefix))
            .cloned()
            .collect();
        let body = json!({"totals": {}, "commit_file_url": "https://x", "files": matching});
        (200, body.to_string())
    })
}

#[test]
fn lists_every_file_with_gaps_under_an_overall_header() {
    let dir = Dir::new("all-text", true);
    let mock = codecov();
    let run = run(&dir.root, &mock.url, &["--all", "-b", "feat"]);
    assert_eq!(run.code, 0, "{run:?}");
    assert_eq!(
        run.stdout,
        "branch feat: 68.18% overall (30/44 lines hit), 3 of 4 files with missing coverage\n\n\
         src/cli.ts: 60% (12/20 lines hit, 8 missed, 0 partial)\n  missed:  2-8, 20\n\n\
         src/index.ts: 50% (5/10 lines hit, 3 missed, 2 partial)\n  missed:  3-5, 9\n  partial: 7, 10\n\n\
         src2/other.ts: 75% (3/4 lines hit, 0 missed, 1 partial)\n  partial: 4\n"
    );
    let requests = mock.requests();
    assert_eq!(requests[0].path, "/api/v2/github/acme/repos/shop/report/");
    assert_eq!(
        requests[0].query.get("branch").map(String::as_str),
        Some("feat")
    );
    assert_eq!(requests[0].query.get("path"), None);
}

#[test]
fn sort_by_misses_and_files_only_list_the_largest_gaps_first() {
    let dir = Dir::new("all-sort", true);
    let mock = codecov();
    let run = run(
        &dir.root,
        &mock.url,
        &["-a", "-l", "--sort", "misses", "--sha", "abc123"],
    );
    assert_eq!(
        run.stdout,
        "commit abc123: 68.18% overall (30/44 lines hit), 3 of 4 files with missing coverage\n\n\
         src/cli.ts: 60% (12/20 lines hit, 8 missed, 0 partial)\n\
         src/index.ts: 50% (5/10 lines hit, 3 missed, 2 partial)\n\
         src2/other.ts: 75% (3/4 lines hit, 0 missed, 1 partial)\n"
    );
    assert_eq!(
        mock.requests()[0].query.get("sha").map(String::as_str),
        Some("abc123")
    );
}

#[test]
fn a_directory_limits_the_report_and_dot_in_a_subdirectory_means_that_directory() {
    let dir = Dir::new("all-prefix", true);
    dir.write("src/index.ts", "");
    let mock = codecov();
    let run = run(&dir.root.join("src"), &mock.url, &["-a", ".", "-l"]);
    assert_eq!(
        run.stdout,
        "default branch (src/): 67.5% overall (27/40 lines hit), 2 of 3 files with missing coverage\n\n\
         src/cli.ts: 60% (12/20 lines hit, 8 missed, 0 partial)\n\
         src/index.ts: 50% (5/10 lines hit, 3 missed, 2 partial)\n"
    );
    assert_eq!(
        mock.requests()[0].query.get("path").map(String::as_str),
        Some("src/")
    );
}

#[test]
fn the_repository_root_as_a_directory_is_the_whole_repository() {
    let dir = Dir::new("all-root", true);
    dir.write("src/index.ts", "");
    let mock = codecov();
    let run = run(&dir.root.join("src"), &mock.url, &["-a", "src", "..", "-l"]);
    assert!(run.stdout.starts_with("default branch: "), "{}", run.stdout);
    assert_eq!(mock.requests().len(), 1);
    assert_eq!(mock.requests()[0].query.get("path"), None);
}

#[test]
fn overlapping_directories_count_each_file_once() {
    let dir = Dir::new("all-overlap", true);
    dir.write("src/index.ts", "");
    let mock = codecov();
    let run = run(&dir.root, &mock.url, &["-a", "src", "src/index.ts", "-l"]);
    assert!(
        run.stdout.starts_with(
            "default branch (src/, src/index.ts): 67.5% overall (27/40 lines hit), 2 of 3"
        ),
        "{}",
        run.stdout
    );
    assert_eq!(mock.requests().len(), 2);
}

#[test]
fn json_has_totals_and_the_files_with_gaps() {
    let dir = Dir::new("all-json", true);
    let mock = codecov();
    let json = run(&dir.root, &mock.url, &["-a", "--json", "-l"]).json();
    assert_eq!(
        json["totals"],
        json!({"coverage": 68.18, "lines": 44, "hits": 30, "misses": 11, "partials": 3,
               "files": 4, "files_with_missing": 3})
    );
    let paths: Vec<_> = json["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].clone())
        .collect();
    assert_eq!(paths, ["src/cli.ts", "src/index.ts", "src2/other.ts"]);
    assert!(json["files"][0].get("missed_lines").is_none());
}

#[test]
fn raw_is_one_report_per_request() {
    let dir = Dir::new("all-raw", true);
    let mock = codecov();
    let json = run(&dir.root, &mock.url, &["-a", "--raw"]).json();
    assert_eq!(json.as_array().unwrap().len(), 1);
    assert_eq!(json[0]["files"].as_array().unwrap().len(), 4);
    assert!(json[0].get("commit_file_url").is_none());
}

#[test]
fn a_failed_request_prints_nothing_and_exits_1() {
    let dir = Dir::new("all-401", true);
    let mock = Mock::start(|_| (401, r#"{"detail":"Invalid token."}"#.to_owned()));
    let run = run(&dir.root, &mock.url, &["-a"]);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert!(run.stderr.contains("401 Unauthorized"), "{}", run.stderr);
}
