//! The `git` binary, for the repository root, the `origin` remote, commit
//! names and the diff. Run in the current directory.

use crate::api::Repo;
use crate::diff::{self, ChangedLines};
use std::path::PathBuf;
use std::process::Command;

/// Runs `git` and returns its stdout, or its stderr as the error.
///
/// # Errors
///
/// `git` is missing, or it exits non-zero.
pub fn run(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        Err(if stderr.is_empty() {
            format!("git {} failed with {}", args.join(" "), output.status)
        } else {
            stderr.to_owned()
        })
    }
}

/// The repository root, or `None` outside a repository.
pub fn toplevel() -> Option<PathBuf> {
    run(&["rev-parse", "--show-toplevel"])
        .ok()
        .map(|out| PathBuf::from(out.trim_end_matches(['\n', '\r'])))
}

/// The full SHA a revision names, or `None` when it names no commit.
pub fn commit(rev: &str) -> Option<String> {
    run(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("{rev}^{{commit}}"),
    ])
    .ok()
    .map(|out| out.trim().to_owned())
    .filter(|sha| !sha.is_empty())
}

/// The lines changed between the merge base of `base` and `head`, and `head`:
/// what a pull request from `head` into `base` shows.
///
/// # Errors
///
/// `git diff` fails.
pub fn changed_lines(base: &str, head: &str) -> Result<ChangedLines, String> {
    let range = format!("{base}...{head}");
    // Every option that changes the format is pinned, so a user's
    // `diff.noprefix`, `diff.external` or `core.quotePath` cannot break the parse.
    let output = run(&[
        "-c",
        "core.quotePath=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--unified=0",
        "--find-renames",
        "--diff-filter=d",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        &range,
    ])?;
    Ok(diff::parse(&output))
}

/// Service, owner and name from the `origin` remote. The service is `None`
/// for a host Codecov does not know by name, such as a self-hosted GitLab.
pub fn origin() -> Option<(Option<String>, String, String)> {
    let url = run(&["remote", "get-url", "origin"]).ok()?;
    parse_remote(url.trim())
}

/// Reads `git@host:owner/repo.git`, `ssh://git@host:22/owner/repo` and
/// `https://user@host/group/sub/repo.git`. The owner keeps every segment but
/// the last, as GitLab subgroups need.
pub fn parse_remote(url: &str) -> Option<(Option<String>, String, String)> {
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        (host.split(':').next()?, path)
    } else {
        // scp-like syntax: [user@]host:path
        let (authority, path) = url.split_once(':')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        (host, path)
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.rsplit_once('/')?;
    if host.is_empty() || owner.is_empty() || name.is_empty() {
        return None;
    }
    let service = match host {
        "github.com" => Some("github"),
        "gitlab.com" => Some("gitlab"),
        "bitbucket.org" => Some("bitbucket"),
        _ => None,
    };
    Some((
        service.map(str::to_owned),
        owner.to_owned(),
        name.to_owned(),
    ))
}

impl Repo {
    /// A repository from the three settings, each falling back to `origin`.
    ///
    /// # Errors
    ///
    /// The name of the first value neither the caller nor `origin` provides.
    pub fn resolve(
        service: Option<String>,
        owner: Option<String>,
        name: Option<String>,
    ) -> Result<Repo, &'static str> {
        let detected = if service.is_some() && owner.is_some() && name.is_some() {
            None
        } else {
            origin()
        };
        let (d_service, d_owner, d_name) = match detected {
            Some((s, o, n)) => (s, Some(o), Some(n)),
            None => (None, None, None),
        };
        Ok(Repo {
            service: service.or(d_service).ok_or("CODECOV_SERVICE")?,
            owner: owner.or(d_owner).ok_or("CODECOV_OWNER")?,
            name: name.or(d_name).ok_or("CODECOV_REPO")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::parse_remote;

    fn parsed(url: &str) -> Option<(Option<&'static str>, String, String)> {
        parse_remote(url).map(|(service, owner, name)| {
            let service = service.map(|s| match s.as_str() {
                "github" => "github",
                "gitlab" => "gitlab",
                "bitbucket" => "bitbucket",
                other => panic!("unexpected service {other}"),
            });
            (service, owner, name)
        })
    }

    #[test]
    fn parse_remote_reads_ssh_https_and_scp_forms() {
        let expected = Some((Some("github"), "acme".to_owned(), "shop".to_owned()));
        assert_eq!(parsed("git@github.com:acme/shop.git"), expected);
        assert_eq!(parsed("https://github.com/acme/shop.git"), expected);
        assert_eq!(parsed("https://github.com/acme/shop"), expected);
        assert_eq!(parsed("https://token@github.com/acme/shop/"), expected);
        assert_eq!(parsed("ssh://git@github.com:22/acme/shop.git"), expected);
    }

    #[test]
    fn parse_remote_keeps_gitlab_subgroups_in_the_owner() {
        assert_eq!(
            parsed("git@gitlab.com:group/sub/repo.git"),
            Some((Some("gitlab"), "group/sub".to_owned(), "repo".to_owned()))
        );
    }

    #[test]
    fn an_unknown_host_has_no_service_but_keeps_owner_and_name() {
        assert_eq!(
            parsed("https://git.example.com/team/app.git"),
            Some((None, "team".to_owned(), "app".to_owned()))
        );
    }

    #[test]
    fn parse_remote_rejects_what_is_not_a_repository_url() {
        assert_eq!(parsed("/local/path/repo"), None);
        assert_eq!(parsed("https://github.com/shop"), None);
        assert_eq!(parsed(""), None);
    }
}
