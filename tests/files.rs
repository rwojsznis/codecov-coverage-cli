//! Named files: one `file_report` request each.

mod common;

use common::{Dir, Mock, Request, run};

/// `src/a.rs` has gaps, `src/full.rs` is covered, anything else is unknown.
fn codecov() -> Mock {
    Mock::start(|request: &Request| {
        let file = request.path.rsplit('/').next().unwrap_or_default();
        match common::decode(file).as_str() {
            "src/a.rs" => (
                200,
                r#"{"name":"src/a.rs","commit_sha":"abc","commit_file_url":"https://x",
                    "totals":{"lines":10,"hits":5,"misses":3,"partials":2,"coverage":50.0},
                    "line_coverage":[[1,0],[3,1],[4,1],[5,1],[7,2],[9,1],[10,2],[12,null]]}"#
                    .to_owned(),
            ),
            "src/full.rs" => (
                200,
                r#"{"name":"src/full.rs","totals":{"lines":2,"hits":2,"misses":0,"partials":0,"coverage":100.0},
                    "line_coverage":[[1,0],[2,0]]}"#
                    .to_owned(),
            ),
            _ => (404, r#"{"detail":"Not found."}"#.to_owned()),
        }
    })
}

#[test]
fn prints_totals_and_missed_line_ranges_per_file() {
    let dir = Dir::new("files-text", true);
    let mock = codecov();
    let run = run(
        &dir.root,
        &mock.url,
        &["src/a.rs", "src/full.rs", "--branch", "main"],
    );
    assert_eq!(run.code, 0, "{run:?}");
    assert_eq!(
        run.stdout,
        "src/a.rs: 50% (5/10 lines hit, 3 missed, 2 partial)\n  \
         missed:  3-5, 9\n  partial: 7, 10\n\n\
         src/full.rs: 100% (2/2 lines hit, 0 missed, 0 partial)\n  fully covered\n"
    );
}

#[test]
fn requests_each_file_as_one_encoded_segment_with_the_token_and_branch() {
    let dir = Dir::new("files-request", true);
    let mock = codecov();
    run(&dir.root, &mock.url, &["src/a.rs", "--branch", "feature/x"]);
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    // The repository comes from the `origin` remote.
    assert_eq!(
        request.path,
        "/api/v2/github/acme/repos/shop/file_report/src%2Fa.rs"
    );
    assert_eq!(
        request.query.get("branch").map(String::as_str),
        Some("feature/x")
    );
    assert_eq!(request.authorization.as_deref(), Some("Bearer test-token"));
}

#[test]
fn a_path_is_resolved_from_the_working_directory_to_the_repository_root() {
    let dir = Dir::new("files-paths", true);
    dir.write("src/a.rs", "");
    let mock = codecov();
    let absolute = dir.root.join("src/full.rs");
    let run = run(
        &dir.root.join("src"),
        &mock.url,
        &["a.rs", "./a.rs", absolute.to_str().unwrap(), "-l"],
    );
    assert_eq!(run.code, 0, "{run:?}");
    let mut files: Vec<String> = mock
        .requests()
        .iter()
        .map(|r| r.path.rsplit('/').next().unwrap().to_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["src%2Fa.rs", "src%2Fa.rs", "src%2Ffull.rs"]);
}

#[test]
fn files_only_prints_one_line_per_file() {
    let dir = Dir::new("files-only", true);
    let mock = codecov();
    let run = run(
        &dir.root,
        &mock.url,
        &["src/a.rs", "src/full.rs", "--files-only"],
    );
    assert_eq!(
        run.stdout,
        "src/a.rs: 50% (5/10 lines hit, 3 missed, 2 partial)\n\
         src/full.rs: 100% (2/2 lines hit, 0 missed, 0 partial)\n"
    );
}

#[test]
fn json_is_an_array_of_summaries_with_line_ranges() {
    let dir = Dir::new("files-json", true);
    let mock = codecov();
    let run = run(&dir.root, &mock.url, &["src/a.rs", "--json"]);
    let json = run.json();
    assert_eq!(json[0]["path"], "src/a.rs");
    assert_eq!(json[0]["misses"], 3);
    assert_eq!(json[0]["missed_lines"], serde_json::json!(["3-5", "9"]));
    assert_eq!(json[0]["partial_lines"], serde_json::json!(["7", "10"]));

    let run = common::run(&dir.root, &mock.url, &["src/a.rs", "--json", "-l"]);
    assert!(run.json()[0].get("missed_lines").is_none());
}

#[test]
fn raw_is_codecov_report_without_the_commit_link_and_sha() {
    let dir = Dir::new("files-raw", true);
    let mock = codecov();
    let run = run(&dir.root, &mock.url, &["src/a.rs", "--raw"]);
    let json = run.json();
    assert_eq!(json[0]["name"], "src/a.rs");
    assert_eq!(json[0]["line_coverage"][7], serde_json::json!([12, null]));
    assert!(json[0].get("commit_sha").is_none());
    assert!(json[0].get("commit_file_url").is_none());
}

#[test]
fn a_file_codecov_does_not_know_fails_alone_with_exit_code_1() {
    let dir = Dir::new("files-404", true);
    let mock = codecov();
    let run = run(
        &dir.root,
        &mock.url,
        &["src/missing.rs", "src/full.rs", "-l"],
    );
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout,
        "src/full.rs: 100% (2/2 lines hit, 0 missed, 0 partial)\n"
    );
    assert!(
        run.stderr
            .starts_with("src/missing.rs: Error fetching coverage report: 404 Not Found\n"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains(r#"{"detail":"Not found."}"#));
}

#[test]
fn a_body_that_is_not_a_report_is_an_error_not_a_panic() {
    let dir = Dir::new("files-garbage", true);
    let mock = Mock::always("<html>maintenance</html>");
    let run = run(&dir.root, &mock.url, &["src/a.rs"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("not JSON"), "{}", run.stderr);

    let mock = Mock::always(r#"{"name":"src/a.rs","line_coverage":"oops"}"#);
    let run = common::run(&dir.root, &mock.url, &["src/a.rs"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("unexpected report"), "{}", run.stderr);
}
