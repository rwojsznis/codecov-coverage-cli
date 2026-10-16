//! The three modes: named files, every file of a commit (`--all`), and the
//! lines a branch changed (`--diff`).

use crate::cli::out::{errln, outln};
use crate::cli::{Exit, Failure};
use codecov_coverage_cli::api::{ApiError, Client, Target};
use codecov_coverage_cli::diff::common_dir_prefix;
use codecov_coverage_cli::git;
use codecov_coverage_cli::report::{self, CoverageReport, FileReport, SortKey, Summary, Totals};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Requests in flight at once. Enough to hide latency for a handful of
/// files, few enough not to look like abuse to the API.
const PARALLEL_REQUESTS: usize = 8;

#[derive(Debug)]
pub(crate) struct Options {
    pub(crate) target: Target,
    pub(crate) json: bool,
    pub(crate) raw: bool,
    pub(crate) files_only: bool,
    pub(crate) sort: SortKey,
}

/// One report per file. A file Codecov cannot report on is an error on stderr
/// and exit code 1, but the other files still print.
pub(crate) fn files(client: &Client, paths: &[String], opts: &Options) -> Exit {
    let results = parallel(paths, |path| client.file_report(path, &opts.target));

    let mut exit = Exit::Ok;
    let mut raw = Vec::new();
    let mut summaries = Vec::new();
    for (path, result) in paths.iter().zip(results) {
        let parsed = result.and_then(|value| {
            if opts.raw {
                return Ok((value, None));
            }
            let file: FileReport = from_value(value.clone())?;
            Ok((value, Some(Summary::of(&file, path, !opts.files_only))))
        });
        match parsed {
            Ok((value, summary)) => {
                raw.push(value);
                summaries.extend(summary);
            }
            Err(e) => {
                errln!("{path}: {e}");
                exit = Exit::NotFetched;
            }
        }
    }

    if opts.raw {
        if !raw.is_empty() {
            print_json(&raw);
        }
    } else if opts.json {
        if !summaries.is_empty() {
            print_json(&summaries);
        }
    } else if !summaries.is_empty() {
        let text: Vec<String> = summaries
            .iter()
            .map(|s| format_summary(s, "lines"))
            .collect();
        outln!("{}", text.join(separator(opts)));
    }
    exit
}

/// Every file with missed or partial lines, from the one report of a commit.
pub(crate) fn all(client: &Client, prefixes: &[String], opts: &Options) -> Result<Exit, Failure> {
    let reports = fetch_reports(client, prefixes, &opts.target)?;
    if opts.raw {
        print_json(&reports);
        return Ok(Exit::Ok);
    }

    let files = merge(reports)?;
    let summaries: Vec<Summary> = files
        .values()
        .map(|file| Summary::of(file, &file.name, !opts.files_only))
        .collect();
    let totals = Totals::of(&summaries);
    let missing = with_missing(summaries, opts.sort);

    if opts.json {
        print_json(&AllJson {
            totals: &totals,
            files: &missing,
        });
        return Ok(Exit::Ok);
    }

    let target = match (&opts.target.sha, &opts.target.branch) {
        (Some(sha), _) => format!("commit {sha}"),
        (None, Some(branch)) => format!("branch {branch}"),
        (None, None) => "default branch".to_owned(),
    };
    let header = format!(
        "{target}{}: {} overall ({}/{} lines hit), {} of {} files with missing coverage",
        scope(prefixes),
        format_pct(totals.coverage),
        totals.hits,
        totals.lines,
        totals.files_with_missing,
        totals.files,
    );
    print_report(&header, &missing, "lines", opts);
    Ok(Exit::Ok)
}

/// Patch coverage: the lines `git diff <base>...<head>` added or modified,
/// matched against Codecov's report for `<head>` itself, so the line numbers
/// of the two agree.
pub(crate) fn diff(
    client: &Client,
    base: &str,
    prefixes: &[String],
    opts: &Options,
) -> Result<Exit, Failure> {
    if git::commit(base).is_none() {
        return Err(Failure::usage(format!("unknown git revision: {base}")));
    }
    let head = head_commit(&opts.target)?;
    let changes = git::changed_lines(base, &head).map_err(Failure::usage)?;

    let in_scope: Vec<&String> = changes
        .keys()
        .filter(|path| prefixes.is_empty() || prefixes.iter().any(|p| path.starts_with(p.as_str())))
        .collect();
    let label = format!(
        "patch {base}...{}{}",
        &head[..head.len().min(10)],
        scope(prefixes)
    );

    let files = if in_scope.is_empty() {
        BTreeMap::new()
    } else {
        let prefix = common_dir_prefix(in_scope.iter().map(|p| p.as_str()));
        let target = Target {
            branch: None,
            sha: Some(head.clone()),
        };
        let report = client
            .report(Some(prefix.as_str()), &target)
            .map_err(|e| no_coverage_hint(e, &head))?;
        merge(vec![report])?
    };

    let mut summaries = Vec::new();
    for path in in_scope {
        if let (Some(file), Some(lines)) = (files.get(path), changes.get(path)) {
            summaries.extend(Summary::of_patch(file, lines, !opts.files_only));
        }
    }
    let totals = Totals::of(&summaries);
    let missing = with_missing(summaries, opts.sort);

    if opts.json {
        print_json(&DiffJson {
            base,
            head: &head,
            totals: &totals,
            files: &missing,
        });
        return Ok(Exit::Ok);
    }
    if totals.lines == 0 {
        outln!("{label}: no coverable lines changed");
        return Ok(Exit::Ok);
    }

    let header = format!(
        "{label}: {} of changed lines covered ({}/{} hit), {} of {} changed files with missing coverage",
        format_pct(totals.coverage),
        totals.hits,
        totals.lines,
        totals.files_with_missing,
        totals.files,
    );
    print_report(&header, &missing, "changed lines", opts);
    Ok(Exit::Ok)
}

/// HEAD, or the commit `--sha` or `--branch` names. A branch that is not
/// checked out locally falls back to its `origin/` counterpart.
fn head_commit(target: &Target) -> Result<String, Failure> {
    let rev = match (&target.sha, &target.branch) {
        (Some(sha), _) => sha.clone(),
        (None, Some(branch)) if git::commit(branch).is_none() => format!("origin/{branch}"),
        (None, Some(branch)) => branch.clone(),
        (None, None) => "HEAD".to_owned(),
    };
    git::commit(&rev).ok_or_else(|| Failure::usage(format!("unknown git revision: {rev}")))
}

fn no_coverage_hint(error: ApiError, head: &str) -> Failure {
    let mut message = error.message;
    if error.status == Some(404) {
        message.push_str(&format!(
            "\nCodecov has no coverage for commit {head}. Push it and wait for CI to upload \
             coverage, or pass --sha/--branch for a commit that has coverage."
        ));
    }
    Failure::not_fetched(message)
}

/// One request per prefix, or one unfiltered request for the whole repository.
fn fetch_reports(
    client: &Client,
    prefixes: &[String],
    target: &Target,
) -> Result<Vec<Value>, Failure> {
    let requested: Vec<Option<&str>> = if prefixes.is_empty() {
        vec![None]
    } else {
        prefixes.iter().map(|p| Some(p.as_str())).collect()
    };
    parallel(&requested, |prefix| client.report(*prefix, target))
        .into_iter()
        .collect::<Result<_, _>>()
        .map_err(|e| Failure::not_fetched(e.message))
}

/// The files of several reports by path. Overlapping prefixes (`src/` and
/// `src/api/`) return the same file twice.
fn merge(reports: Vec<Value>) -> Result<BTreeMap<String, FileReport>, Failure> {
    let mut files = BTreeMap::new();
    for value in reports {
        let report: CoverageReport =
            from_value(value).map_err(|e| Failure::not_fetched(e.message))?;
        for file in report.files {
            files.insert(file.name.clone(), file);
        }
    }
    Ok(files)
}

fn from_value<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ApiError> {
    serde_json::from_value(value).map_err(|e| ApiError {
        status: None,
        message: format!("Codecov answered with an unexpected report: {e}"),
    })
}

fn with_missing(summaries: Vec<Summary>, sort: SortKey) -> Vec<Summary> {
    let mut missing: Vec<Summary> = summaries.into_iter().filter(Summary::has_missing).collect();
    report::sort(&mut missing, sort);
    missing
}

/// Runs `fetch` for every item, a few at a time, and keeps the input order.
fn parallel<T: Sync, R: Send>(items: &[T], fetch: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let fetch = &fetch;
    let mut results = Vec::with_capacity(items.len());
    for chunk in items.chunks(PARALLEL_REQUESTS) {
        std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|item| scope.spawn(move || fetch(item)))
                .collect();
            for handle in handles {
                match handle.join() {
                    Ok(result) => results.push(result),
                    // A panic in a request is a bug; let it surface as one.
                    Err(payload) => std::panic::resume_unwind(payload),
                }
            }
        });
    }
    results
}

#[derive(Serialize)]
struct AllJson<'a> {
    totals: &'a Totals,
    files: &'a [Summary],
}

#[derive(Serialize)]
struct DiffJson<'a> {
    base: &'a str,
    head: &'a str,
    totals: &'a Totals,
    files: &'a [Summary],
}

fn print_json(value: &impl Serialize) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => outln!("{json}"),
        Err(e) => errln!("codecov-coverage-cli: cannot encode JSON: {e}"),
    }
}

fn print_report(header: &str, files: &[Summary], unit: &str, opts: &Options) {
    if files.is_empty() {
        outln!("{header}");
        return;
    }
    let body: Vec<String> = files.iter().map(|s| format_summary(s, unit)).collect();
    outln!("{header}\n\n{}", body.join(separator(opts)));
}

/// A blank line between files with line ranges; none in a `--files-only` list.
fn separator(opts: &Options) -> &'static str {
    if opts.files_only { "\n" } else { "\n\n" }
}

fn scope(prefixes: &[String]) -> String {
    if prefixes.is_empty() {
        String::new()
    } else {
        format!(" ({})", prefixes.join(", "))
    }
}

fn format_pct(coverage: Option<f64>) -> String {
    coverage.map_or_else(|| "n/a".to_owned(), |c| format!("{c}%"))
}

/// `path: 50% (5/10 lines hit, 3 missed, 2 partial)`, then the ranges.
fn format_summary(s: &Summary, unit: &str) -> String {
    let mut text = format!(
        "{}: {} ({}/{} {unit} hit, {} missed, {} partial)",
        s.path,
        format_pct(s.coverage),
        s.hits,
        s.lines,
        s.misses,
        s.partials
    );
    let (Some(missed), Some(partial)) = (&s.missed_lines, &s.partial_lines) else {
        return text;
    };
    if missed.is_empty() && partial.is_empty() {
        text.push_str("\n  fully covered");
    }
    if !missed.is_empty() {
        text.push_str(&format!("\n  missed:  {}", missed.join(", ")));
    }
    if !partial.is_empty() {
        text.push_str(&format!("\n  partial: {}", partial.join(", ")));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(missed: Option<Vec<&str>>, partial: Option<Vec<&str>>) -> Summary {
        let owned = |v: Option<Vec<&str>>| v.map(|v| v.into_iter().map(String::from).collect());
        Summary {
            path: "src/a.rs".into(),
            coverage: Some(50.0),
            lines: 10,
            hits: 5,
            misses: 3,
            partials: 2,
            missed_lines: owned(missed),
            partial_lines: owned(partial),
        }
    }

    #[test]
    fn a_summary_prints_totals_then_ranges() {
        let s = summary(Some(vec!["3-5", "9"]), Some(vec!["7"]));
        assert_eq!(
            format_summary(&s, "lines"),
            "src/a.rs: 50% (5/10 lines hit, 3 missed, 2 partial)\n  missed:  3-5, 9\n  partial: 7"
        );
    }

    #[test]
    fn a_covered_file_says_so_and_files_only_prints_one_line() {
        let s = summary(Some(vec![]), Some(vec![]));
        assert!(format_summary(&s, "lines").ends_with("\n  fully covered"));
        let s = summary(None, None);
        assert_eq!(
            format_summary(&s, "changed lines"),
            "src/a.rs: 50% (5/10 changed lines hit, 3 missed, 2 partial)"
        );
    }

    #[test]
    fn a_percentage_drops_a_zero_fraction() {
        assert_eq!(format_pct(Some(100.0)), "100%");
        assert_eq!(format_pct(Some(68.18)), "68.18%");
        assert_eq!(format_pct(None), "n/a");
    }

    #[test]
    fn parallel_keeps_the_input_order() {
        let items: Vec<u64> = (0..20).collect();
        let doubled = parallel(&items, |n| n * 2);
        assert_eq!(doubled, items.iter().map(|n| n * 2).collect::<Vec<_>>());
    }
}
