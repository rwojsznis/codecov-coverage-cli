//! The command line: the flags, the configuration, and the dispatch to one of
//! the three modes in `modes`.

pub(crate) mod modes;
pub(crate) mod out;

use clap::{Parser, ValueEnum};
use codecov_coverage_cli::api::{Client, DEFAULT_API_URL, Repo, Target};
use codecov_coverage_cli::report::SortKey;
use codecov_coverage_cli::{git, paths};
use std::process::ExitCode;

const VERSION: &str = match option_env!("CODECOV_COVERAGE_CLI_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

const USAGE: &str = "\
codecov-coverage-cli <FILE>... [OPTIONS]
       codecov-coverage-cli --all [DIR]... [OPTIONS]
       codecov-coverage-cli --diff <BASE> [DIR]... [OPTIONS]";

const AFTER_HELP: &str = "\
Environment:
  CODECOV_AUTH_TOKEN  Codecov API token (required)
  CODECOV_SERVICE, CODECOV_OWNER, CODECOV_REPO
                      Default to the \"origin\" remote when it is on github.com,
                      gitlab.com or bitbucket.org

Exit codes: 0 success, 1 a report could not be fetched, 2 usage or configuration error.";

#[derive(Parser)]
#[command(
    name = "codecov-coverage-cli",
    version = VERSION,
    about = "Print the lines Codecov reports as not covered: per file, for the whole \
             repository, or for only the lines a branch changed.",
    long_about = "Print the lines Codecov reports as not covered: per file, for the whole \
                  repository, or for only the lines a branch changed.\n\n\
                  Paths may be absolute or relative to the current directory; inside a git \
                  repository they are resolved relative to its root.",
    override_usage = USAGE,
    after_help = AFTER_HELP
)]
struct Cli {
    /// Files to report on. With --all or --diff: directories to limit the report to.
    #[arg(value_name = "PATH", required_unless_present_any = ["all", "diff"])]
    paths: Vec<String>,

    /// List every file with missed or partial lines.
    #[arg(short, long, conflicts_with = "diff")]
    all: bool,

    /// Only the lines changed since BASE (git diff BASE...head), like Codecov's
    /// patch coverage. The head is HEAD, or --sha / --branch resolved in the
    /// local repository; Codecov must have coverage for that exact commit.
    #[arg(short, long, value_name = "BASE")]
    diff: Option<String>,

    /// Omit line ranges; print only per-file totals.
    #[arg(short = 'l', long)]
    files_only: bool,

    /// Order of the files with --all and --diff.
    #[arg(long, value_enum, default_value_t = Sort::Path)]
    sort: Sort,

    /// Branch to report on [default: the repository's default branch].
    #[arg(short, long, value_name = "NAME")]
    branch: Option<String>,

    /// Commit SHA to report on.
    #[arg(short, long, value_name = "SHA", conflicts_with = "branch")]
    sha: Option<String>,

    /// Print JSON summaries: totals and missed/partial line ranges.
    #[arg(long, conflicts_with = "raw")]
    json: bool,

    /// Print Codecov's reports as they come, as a JSON array.
    #[arg(long, conflicts_with = "diff")]
    raw: bool,

    /// Git hosting service: github, gitlab, bitbucket, or an _enterprise / _server form.
    #[arg(long, value_name = "NAME", env = "CODECOV_SERVICE")]
    service: Option<String>,

    /// Repository owner (user, organization or group).
    #[arg(long, value_name = "NAME", env = "CODECOV_OWNER")]
    owner: Option<String>,

    /// Repository name.
    #[arg(long, value_name = "NAME", env = "CODECOV_REPO")]
    repo: Option<String>,

    /// Codecov API root, for a self-hosted Codecov.
    #[arg(long, value_name = "URL", env = "CODECOV_API_URL", default_value = DEFAULT_API_URL)]
    api_url: String,
}

#[derive(Clone, Copy, ValueEnum)]
enum Sort {
    /// By path.
    Path,
    /// Most missed and partial lines first.
    Misses,
}

/// The process exit code, as a type, so a mode cannot return a bare number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exit {
    Ok,
    /// Codecov had no report, or could not be reached.
    NotFetched,
    /// The command line, the configuration or the git state is wrong.
    Usage,
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> ExitCode {
        match exit {
            Exit::Ok => ExitCode::SUCCESS,
            Exit::NotFetched => ExitCode::from(1),
            Exit::Usage => ExitCode::from(2),
        }
    }
}

/// A run that stops with one message. `main` prints it and exits with `exit`.
#[derive(Debug)]
pub(crate) struct Failure {
    pub(crate) exit: Exit,
    pub(crate) message: String,
}

impl Failure {
    pub(crate) fn usage(message: impl Into<String>) -> Failure {
        Failure {
            exit: Exit::Usage,
            message: message.into(),
        }
    }

    pub(crate) fn not_fetched(message: impl Into<String>) -> Failure {
        Failure {
            exit: Exit::NotFetched,
            message: message.into(),
        }
    }
}

/// Parses the command line and runs the mode it names.
pub(crate) fn run() -> Result<Exit, Failure> {
    Cli::parse().run()
}

impl Cli {
    fn run(self) -> Result<Exit, Failure> {
        let token = non_empty(std::env::var("CODECOV_AUTH_TOKEN").ok())
            .ok_or_else(|| Failure::usage("CODECOV_AUTH_TOKEN is not set"))?;
        let repo = Repo::resolve(
            non_empty(self.service),
            non_empty(self.owner),
            non_empty(self.repo),
        )
        .map_err(|missing| {
            Failure::usage(format!(
                "cannot tell the repository: set {missing}, or run inside a clone whose \
                 \"origin\" remote is on github.com, gitlab.com or bitbucket.org"
            ))
        })?;
        let client = Client::new(&self.api_url, &token, &repo);

        let cwd = std::env::current_dir()
            .map_err(|e| Failure::usage(format!("cannot read the current directory: {e}")))?;
        let root = git::toplevel();
        let opts = modes::Options {
            target: Target {
                branch: self.branch,
                sha: self.sha,
            },
            json: self.json,
            raw: self.raw,
            files_only: self.files_only,
            sort: match self.sort {
                Sort::Path => SortKey::Path,
                Sort::Misses => SortKey::Misses,
            },
        };

        if self.all || self.diff.is_some() {
            let prefixes: Vec<String> = self
                .paths
                .iter()
                .map(|p| paths::to_repo_prefix(p, &cwd, root.as_deref()))
                .collect();
            // The repository root among them means the whole repository.
            let prefixes = if prefixes.iter().any(String::is_empty) {
                Vec::new()
            } else {
                prefixes
            };
            return match self.diff {
                Some(base) => modes::diff(&client, &base, &prefixes, &opts),
                None => modes::all(&client, &prefixes, &opts),
            };
        }

        let files: Vec<String> = self
            .paths
            .iter()
            .map(|p| paths::to_repo_path(p, &cwd, root.as_deref()))
            .collect();
        Ok(modes::files(&client, &files, &opts))
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{Cli, VERSION};
    use clap::CommandFactory;

    #[test]
    fn version_falls_back_to_the_cargo_package_version() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
    }

    /// clap's own consistency check: a duplicated flag, a short option claimed
    /// twice, a conflict that names no argument.
    #[test]
    fn the_command_is_well_formed() {
        Cli::command().debug_assert();
    }
}
