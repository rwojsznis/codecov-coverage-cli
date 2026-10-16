//! `--diff`: patch coverage from a real `git diff` against a stand-in Codecov.

mod common;

use common::{Dir, Mock, Request, run};
use serde_json::json;

/// `main` has a base commit; `feature` edits, renames, adds and deletes
/// files on top of it. Returns the repository and the head of `feature`.
fn repository(name: &str) -> (Dir, String) {
    let dir = Dir::new(name, true);
    let numbered = |word: &str, n: u32| {
        (1..=n).fold(String::new(), |mut text, i| {
            text.push_str(&format!("{word} {i}\n"));
            text
        })
    };
    dir.write("app/models/order.rb", &numbered("line", 20));
    dir.write("app/models/old_name.rb", &numbered("x", 10));
    dir.write("lib/gone.rb", "1\n2\n3\n");
    dir.write("docs/readme.md", "hi\n");
    dir.commit("base");

    dir.git(&["checkout", "--quiet", "-b", "feature"]);
    let mut order = numbered("line", 20)
        .replace("line 5\n", "line 5 changed\n")
        .replace("line 12\n", "line 12 changed\n")
        .replace("line 13\n", "line 13 changed\n");
    order = order.replace("line 18\n", "line 18\ninserted\n");
    dir.write("app/models/order.rb", &order);
    dir.git(&["mv", "app/models/old_name.rb", "app/models/new_name.rb"]);
    dir.write(
        "app/models/new_name.rb",
        &numbered("x", 10).replace("x 3\n", "x 3 edited\n"),
    );
    dir.git(&["rm", "--quiet", "lib/gone.rb"]);
    dir.write("lib/new.rb", "a\nb\nc\n");
    dir.write("lib/café.rb", "u1\nu2\n");
    dir.write("docs/readme.md", "hi\nmore\n");
    let head = dir.commit("feature");
    (dir, head)
}

/// Coverage for `head` only; any other commit is a 404, as on Codecov.
fn codecov(head: &str) -> Mock {
    let head = head.to_owned();
    let files = json!([
        {"name": "app/models/order.rb", "totals": {}, "line_coverage": [[1,1],[5,0],[12,1],[13,2],[15,null],[19,1]]},
        {"name": "app/models/new_name.rb", "totals": {}, "line_coverage": [[3,0],[4,1]]},
        {"name": "lib/new.rb", "totals": {}, "line_coverage": [[1,0],[2,1],[3,null]]},
        {"name": "lib/café.rb", "totals": {}, "line_coverage": [[1,1]]},
        {"name": "other/untouched.rb", "totals": {}, "line_coverage": [[1,1]]},
    ]);
    Mock::start(move |request: &Request| {
        if request.query.get("sha") != Some(&head) {
            return (404, r#"{"detail":"Not found."}"#.to_owned());
        }
        let prefix = request.query.get("path").cloned().unwrap_or_default();
        let matching: Vec<_> = files
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["name"].as_str().unwrap().starts_with(&prefix))
            .cloned()
            .collect();
        (200, json!({"totals": {}, "files": matching}).to_string())
    })
}

#[test]
fn reports_only_the_changed_coverable_lines() {
    let (dir, head) = repository("diff-text");
    let mock = codecov(&head);
    let run = run(&dir.root, &mock.url, &["--diff", "main"]);
    assert_eq!(run.code, 0, "{run:?}");
    // order.rb: line 1 is missed but unchanged, line 15 is not coverable.
    // new_name.rb: the renamed file's one edit is covered, so it is not listed.
    assert_eq!(
        run.stdout,
        format!(
            "patch main...{}: 37.5% of changed lines covered (3/8 hit), \
             3 of 4 changed files with missing coverage\n\n\
             app/models/order.rb: 25% (1/4 changed lines hit, 2 missed, 1 partial)\n  \
             missed:  12, 19\n  partial: 13\n\n\
             lib/café.rb: 0% (0/1 changed lines hit, 1 missed, 0 partial)\n  missed:  1\n\n\
             lib/new.rb: 50% (1/2 changed lines hit, 1 missed, 0 partial)\n  missed:  2\n",
            &head[..10]
        )
    );
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].query.get("sha"), Some(&head));
}

#[test]
fn a_directory_limits_the_patch_and_the_request() {
    let (dir, head) = repository("diff-prefix");
    let mock = codecov(&head);
    let run = run(
        &dir.root.join("app"),
        &mock.url,
        &["-d", "main", "../lib", "-l"],
    );
    assert_eq!(
        run.stdout,
        format!(
            "patch main...{} (lib/): 33.33% of changed lines covered (1/3 hit), \
             2 of 2 changed files with missing coverage\n\n\
             lib/café.rb: 0% (0/1 changed lines hit, 1 missed, 0 partial)\n\
             lib/new.rb: 50% (1/2 changed lines hit, 1 missed, 0 partial)\n",
            &head[..10]
        )
    );
    assert_eq!(
        mock.requests()[0].query.get("path").map(String::as_str),
        Some("lib/")
    );
}

#[test]
fn the_request_asks_only_for_the_directory_the_changes_share() {
    let (dir, head) = repository("diff-common");
    dir.write("app/models/order.rb", "changed again\n");
    let head2 = dir.commit("only app/models");
    let mock = codecov(&head2);
    run(&dir.root, &mock.url, &["-d", &head]);
    assert_eq!(
        mock.requests()[0].query.get("path").map(String::as_str),
        Some("app/models/")
    );
}

#[test]
fn json_names_base_and_head_and_lists_changed_lines_only() {
    let (dir, head) = repository("diff-json");
    let mock = codecov(&head);
    let json = run(
        &dir.root,
        &mock.url,
        &["-d", "main", "--json", "--sort", "misses"],
    )
    .json();
    assert_eq!(json["base"], "main");
    assert_eq!(json["head"], head.as_str());
    assert_eq!(
        json["totals"],
        json!({"coverage": 37.5, "lines": 8, "hits": 3, "misses": 4, "partials": 1,
               "files": 4, "files_with_missing": 3})
    );
    assert_eq!(json["files"][0]["path"], "app/models/order.rb");
    assert_eq!(json["files"][0]["missed_lines"], json!(["12", "19"]));
}

#[test]
fn a_head_without_coverage_explains_what_to_do_and_exits_1() {
    let (dir, head) = repository("diff-404");
    // A commit CI has not uploaded coverage for yet.
    dir.write("lib/new.rb", "a\nb\nc\nd\n");
    let unpushed = dir.commit("unpushed");
    let mock = codecov(&head);
    let run = run(&dir.root, &mock.url, &["-d", "main"]);
    assert_eq!(mock.requests()[0].query.get("sha"), Some(&unpushed));
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert!(run.stderr.contains("404 Not Found"), "{}", run.stderr);
    assert!(
        run.stderr.contains("Codecov has no coverage for commit"),
        "{}",
        run.stderr
    );
}

#[test]
fn an_unknown_base_is_a_usage_error() {
    let (dir, head) = repository("diff-unknown");
    let mock = codecov(&head);
    let run = run(&dir.root, &mock.url, &["-d", "nope"]);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        "codecov-coverage-cli: unknown git revision: nope\n"
    );
    assert!(mock.requests().is_empty());
}

#[test]
fn a_branch_that_is_not_checked_out_falls_back_to_origin() {
    let (dir, head) = repository("diff-origin");
    dir.git(&["update-ref", "refs/remotes/origin/pushed", &head]);
    dir.git(&["checkout", "--quiet", "main"]);
    let mock = codecov(&head);
    let run = run(&dir.root, &mock.url, &["-d", "main", "-b", "pushed", "-l"]);
    assert_eq!(run.code, 0, "{run:?}");
    assert!(
        run.stdout.contains("3 of 4 changed files"),
        "{}",
        run.stdout
    );

    let run = common::run(&dir.root, &mock.url, &["-d", "main", "-b", "never"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains("unknown git revision: origin/never"),
        "{}",
        run.stderr
    );
}

#[test]
fn no_changes_means_no_coverable_lines_and_no_request() {
    let (dir, head) = repository("diff-empty");
    let mock = codecov(&head);
    let run = run(&dir.root, &mock.url, &["-d", "HEAD"]);
    assert_eq!(run.code, 0, "{run:?}");
    assert_eq!(
        run.stdout,
        format!("patch HEAD...{}: no coverable lines changed\n", &head[..10])
    );
    assert!(mock.requests().is_empty());
}

#[test]
fn the_users_git_diff_settings_do_not_change_the_parse() {
    let (dir, head) = repository("diff-config");
    let mock = codecov(&head);
    let run = common::finish(
        common::command(&dir.root, &mock.url)
            .args(["-d", "main", "-l"])
            .env("GIT_CONFIG_COUNT", "3")
            .env("GIT_CONFIG_KEY_0", "diff.noprefix")
            .env("GIT_CONFIG_VALUE_0", "true")
            .env("GIT_CONFIG_KEY_1", "diff.mnemonicPrefix")
            .env("GIT_CONFIG_VALUE_1", "true")
            .env("GIT_CONFIG_KEY_2", "core.quotePath")
            .env("GIT_CONFIG_VALUE_2", "true"),
    );
    assert_eq!(run.code, 0, "{run:?}");
    assert!(run.stdout.contains("lib/café.rb: 0%"), "{}", run.stdout);
    assert!(
        run.stdout.contains("3 of 4 changed files"),
        "{}",
        run.stdout
    );
}

#[test]
fn sha_or_a_local_branch_names_the_head_instead_of_head() {
    let (dir, head) = repository("diff-head");
    dir.git(&["checkout", "--quiet", "main"]);
    let mock = codecov(&head);
    for args in [
        ["-d", "main", "--sha", &head[..12]],
        ["-d", "main", "--branch", "feature"],
    ] {
        let run = run(&dir.root, &mock.url, &args);
        assert_eq!(run.code, 0, "{args:?}: {run:?}");
        assert!(
            run.stdout.contains("3 of 4 changed files"),
            "{}",
            run.stdout
        );
    }
    // A short SHA reaches Codecov as the full one.
    assert!(
        mock.requests()
            .iter()
            .all(|r| r.query.get("sha") == Some(&head))
    );
}
