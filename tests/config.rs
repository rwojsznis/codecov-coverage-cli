//! Configuration, the command line itself, and exit codes: 0 success, 1 a
//! report could not be fetched, 2 usage or configuration error.

mod common;

use common::{Dir, Mock, command, finish, run};

const REPORT: &str = r#"{"name":"a.rs","totals":{"lines":1,"hits":1},"line_coverage":[[1,0]]}"#;

#[test]
fn a_missing_token_is_a_configuration_error() {
    let dir = Dir::new("config-token", true);
    let mock = Mock::always(REPORT);
    let run = finish(
        command(&dir.root, &mock.url)
            .arg("a.rs")
            .env_remove("CODECOV_AUTH_TOKEN"),
    );
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        "codecov-coverage-cli: CODECOV_AUTH_TOKEN is not set\n"
    );
    assert!(mock.requests().is_empty());
}

#[test]
fn outside_a_repository_the_repository_must_be_configured() {
    let dir = Dir::new("config-norepo", false);
    let mock = Mock::always(REPORT);
    let run = run(&dir.root, &mock.url, &["a.rs"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("set CODECOV_SERVICE"), "{}", run.stderr);

    let run = finish(
        command(&dir.root, &mock.url)
            .arg("./a.rs")
            .env("CODECOV_SERVICE", "gitlab")
            .env("CODECOV_OWNER", "group/sub")
            .env("CODECOV_REPO", "app"),
    );
    assert_eq!(run.code, 0, "{run:?}");
    assert_eq!(
        mock.requests()[0].path,
        "/api/v2/gitlab/group%2Fsub/repos/app/file_report/a.rs"
    );
}

#[test]
fn flags_override_the_environment_and_the_origin_remote() {
    let dir = Dir::new("config-flags", true);
    let mock = Mock::always(REPORT);
    let run = finish(
        command(&dir.root, &mock.url)
            .args(["a.rs", "--service", "bitbucket", "--owner", "team"])
            .env("CODECOV_OWNER", "ignored"),
    );
    assert_eq!(run.code, 0, "{run:?}");
    // The repository name still comes from `origin`.
    assert_eq!(
        mock.requests()[0].path,
        "/api/v2/bitbucket/team/repos/shop/file_report/a.rs"
    );
}

#[test]
fn an_unknown_host_needs_the_service_but_keeps_owner_and_name() {
    let dir = Dir::new("config-host", true);
    dir.git(&[
        "remote",
        "set-url",
        "origin",
        "https://git.example.com/team/app.git",
    ]);
    let mock = Mock::always(REPORT);
    let run = run(&dir.root, &mock.url, &["a.rs"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("set CODECOV_SERVICE"), "{}", run.stderr);

    let run = common::run(
        &dir.root,
        &mock.url,
        &["a.rs", "--service", "gitlab_enterprise"],
    );
    assert_eq!(run.code, 0, "{run:?}");
    assert_eq!(
        mock.requests()[0].path,
        "/api/v2/gitlab_enterprise/team/repos/app/file_report/a.rs"
    );
}

#[test]
fn usage_errors_exit_2() {
    let dir = Dir::new("config-usage", true);
    let mock = Mock::always(REPORT);
    for args in [
        vec![],
        vec!["--all", "--diff", "main"],
        vec!["a.rs", "--json", "--raw"],
        vec!["--diff", "main", "--raw"],
        vec!["a.rs", "--sha", "abc", "--branch", "main"],
        vec!["--all", "--sort", "size"],
    ] {
        let run = run(&dir.root, &mock.url, &args);
        assert_eq!(run.code, 2, "{args:?}: {run:?}");
        assert!(run.stderr.starts_with("error:"), "{args:?}: {}", run.stderr);
    }
    assert!(mock.requests().is_empty());
}

#[test]
fn an_unreachable_api_exits_1() {
    let dir = Dir::new("config-unreachable", true);
    // Bind and drop: the port is closed by the time the CLI connects.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let run = run(&dir.root, &format!("http://127.0.0.1:{port}"), &["a.rs"]);
    assert_eq!(run.code, 1, "{run:?}");
    assert!(run.stderr.starts_with("a.rs: "), "{}", run.stderr);
}

#[test]
fn a_self_hosted_api_url_is_used_as_the_root() {
    let dir = Dir::new("config-api-url", true);
    let mock = Mock::always(REPORT);
    let run = run(&dir.root, &format!("{}/", mock.url), &["a.rs"]);
    assert_eq!(run.code, 0, "{run:?}");
    assert_eq!(
        mock.requests()[0].path,
        "/api/v2/github/acme/repos/shop/file_report/a.rs"
    );
}

#[test]
fn help_and_version_exit_0() {
    let dir = Dir::new("config-help", false);
    let run = run(&dir.root, "http://unused", &["--help"]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("--diff <BASE>"), "{}", run.stdout);
    assert!(run.stdout.contains("CODECOV_AUTH_TOKEN"), "{}", run.stdout);

    let run = common::run(&dir.root, "http://unused", &["--version"]);
    assert_eq!(run.code, 0);
    assert_eq!(
        run.stdout,
        format!("codecov-coverage-cli {}\n", env!("CARGO_PKG_VERSION"))
    );
}
