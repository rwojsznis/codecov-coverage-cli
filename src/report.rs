//! Codecov's reports, reduced to what an agent acts on: per-file totals and
//! the missed and partial lines as compact ranges.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;

/// The values of a `line_coverage` entry.
/// ref: <https://docs.codecov.com/reference/repos_report_retrieve> ("hit=0/miss=1/partial=2")
pub const HIT: u8 = 0;
pub const MISS: u8 = 1;
pub const PARTIAL: u8 = 2;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReportTotals {
    #[serde(default, deserialize_with = "nullable")]
    pub lines: u64,
    #[serde(default, deserialize_with = "nullable")]
    pub hits: u64,
    #[serde(default, deserialize_with = "nullable")]
    pub misses: u64,
    #[serde(default, deserialize_with = "nullable")]
    pub partials: u64,
    #[serde(default)]
    pub coverage: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FileReport {
    #[serde(default, deserialize_with = "nullable")]
    pub name: String,
    #[serde(default, deserialize_with = "nullable")]
    pub totals: ReportTotals,
    /// `[line, code]` pairs. A `null` code is a line Codecov does not count.
    #[serde(default, deserialize_with = "nullable")]
    pub line_coverage: Vec<(u64, Option<u8>)>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CoverageReport {
    #[serde(default, deserialize_with = "nullable")]
    pub files: Vec<FileReport>,
}

/// Codecov sends `null` for some fields it has no value for. `#[serde(default)]`
/// only covers a missing field, so `null` maps to the default here.
fn nullable<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

/// One file, as the CLI prints it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub path: String,
    pub coverage: Option<f64>,
    pub lines: u64,
    pub hits: u64,
    pub misses: u64,
    pub partials: u64,
    /// `None` under `--files-only`, so the JSON drops the key rather than
    /// claiming an empty list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missed_lines: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_lines: Option<Vec<String>>,
}

impl Summary {
    /// The file as Codecov totals it. `fallback_path` names it when the report
    /// does not.
    pub fn of(file: &FileReport, fallback_path: &str, with_lines: bool) -> Summary {
        let path = if file.name.is_empty() {
            fallback_path
        } else {
            &file.name
        };
        Summary {
            path: path.to_owned(),
            coverage: file.totals.coverage,
            lines: file.totals.lines,
            hits: file.totals.hits,
            misses: file.totals.misses,
            partials: file.totals.partials,
            missed_lines: with_lines.then(|| lines_with(&file.line_coverage, MISS)),
            partial_lines: with_lines.then(|| lines_with(&file.line_coverage, PARTIAL)),
        }
    }

    /// The file restricted to `changed`: only the lines a diff added or
    /// modified, and only those Codecov counts. `None` when no changed line is
    /// coverable, so a changed README or a comment-only edit drops out.
    pub fn of_patch(
        file: &FileReport,
        changed: &BTreeSet<u64>,
        with_lines: bool,
    ) -> Option<Summary> {
        let line_coverage: Vec<(u64, Option<u8>)> = file
            .line_coverage
            .iter()
            .copied()
            .filter(|&(line, code)| code.is_some() && changed.contains(&line))
            .collect();
        if line_coverage.is_empty() {
            return None;
        }
        let count = |wanted: u8| count_with(&line_coverage, wanted);
        let lines = line_coverage.len() as u64;
        let hits = count(HIT);
        Some(Summary {
            path: file.name.clone(),
            coverage: percentage(hits, lines),
            lines,
            hits,
            misses: count(MISS),
            partials: count(PARTIAL),
            missed_lines: with_lines.then(|| lines_with(&line_coverage, MISS)),
            partial_lines: with_lines.then(|| lines_with(&line_coverage, PARTIAL)),
        })
    }

    pub fn has_missing(&self) -> bool {
        self.misses + self.partials > 0
    }
}

fn lines_with(line_coverage: &[(u64, Option<u8>)], wanted: u8) -> Vec<String> {
    to_ranges(
        line_coverage
            .iter()
            .filter(|&&(_, code)| code == Some(wanted))
            .map(|&(line, _)| line),
    )
}

fn count_with(line_coverage: &[(u64, Option<u8>)], wanted: u8) -> u64 {
    line_coverage
        .iter()
        .filter(|&&(_, code)| code == Some(wanted))
        .count() as u64
}

/// Sums over every file, including the fully covered ones, so the header can
/// say "3 of 40 files".
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Totals {
    pub coverage: Option<f64>,
    pub lines: u64,
    pub hits: u64,
    pub misses: u64,
    pub partials: u64,
    pub files: usize,
    pub files_with_missing: usize,
}

impl Totals {
    pub fn of(files: &[Summary]) -> Totals {
        let sum = |field: fn(&Summary) -> u64| files.iter().map(field).sum::<u64>();
        let lines = sum(|f| f.lines);
        let hits = sum(|f| f.hits);
        Totals {
            coverage: percentage(hits, lines),
            lines,
            hits,
            misses: sum(|f| f.misses),
            partials: sum(|f| f.partials),
            files: files.len(),
            files_with_missing: files.iter().filter(|f| f.has_missing()).count(),
        }
    }
}

/// Codecov's definition: hits over all coverable lines, partials included in
/// the denominator. Rounded to two decimals, as Codecov shows it.
#[allow(
    clippy::cast_precision_loss,
    reason = "line counts stay far below 2^52, where f64 stops being exact"
)]
pub fn percentage(hits: u64, lines: u64) -> Option<f64> {
    (lines > 0).then(|| (hits as f64 / lines as f64 * 10_000.0).round() / 100.0)
}

/// `1, 2, 3, 5` → `["1-3", "5"]`. Sorts and deduplicates first.
pub fn to_ranges(lines: impl IntoIterator<Item = u64>) -> Vec<String> {
    let sorted: BTreeSet<u64> = lines.into_iter().collect();
    let mut ranges = Vec::new();
    let mut iter = sorted.into_iter();
    let Some(mut start) = iter.next() else {
        return ranges;
    };
    let mut end = start;
    for line in iter {
        if line == end + 1 {
            end = line;
            continue;
        }
        ranges.push(range(start, end));
        start = line;
        end = line;
    }
    ranges.push(range(start, end));
    ranges
}

fn range(start: u64, end: u64) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Path,
    /// Most missed + partial lines first; ties by path.
    Misses,
}

pub fn sort(files: &mut [Summary], key: SortKey) {
    match key {
        SortKey::Path => files.sort_by(|a, b| a.path.cmp(&b.path)),
        SortKey::Misses => files.sort_by(|a, b| {
            (b.misses + b.partials)
                .cmp(&(a.misses + a.partials))
                .then_with(|| a.path.cmp(&b.path))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, line_coverage: &[(u64, Option<u8>)]) -> FileReport {
        FileReport {
            name: name.into(),
            totals: ReportTotals {
                lines: 10,
                hits: 5,
                misses: 3,
                partials: 2,
                coverage: Some(50.0),
            },
            line_coverage: line_coverage.to_vec(),
        }
    }

    #[test]
    fn to_ranges_merges_consecutive_lines() {
        assert_eq!(to_ranges([9, 3, 4, 5, 9, 12]), ["3-5", "9", "12"]);
        assert_eq!(to_ranges([]), Vec::<String>::new());
        assert_eq!(to_ranges([7]), ["7"]);
    }

    #[test]
    fn a_summary_keeps_codecov_totals_and_lists_missed_and_partial_lines() {
        let f = file(
            "a.rs",
            &[
                (1, Some(0)),
                (3, Some(1)),
                (4, Some(1)),
                (7, Some(2)),
                (8, None),
            ],
        );
        let s = Summary::of(&f, "ignored", true);
        assert_eq!(s.path, "a.rs");
        assert_eq!((s.lines, s.hits, s.misses, s.partials), (10, 5, 3, 2));
        assert_eq!(s.missed_lines.unwrap(), ["3-4"]);
        assert_eq!(s.partial_lines.unwrap(), ["7"]);
    }

    #[test]
    fn files_only_leaves_the_line_lists_out() {
        let s = Summary::of(&file("a.rs", &[(3, Some(1))]), "a.rs", false);
        assert_eq!(s.missed_lines, None);
        let json = serde_json::to_value(&s).unwrap();
        assert!(json.get("missed_lines").is_none());
    }

    #[test]
    fn a_report_without_a_name_uses_the_requested_path() {
        let s = Summary::of(&file("", &[]), "src/a.rs", true);
        assert_eq!(s.path, "src/a.rs");
    }

    #[test]
    fn a_patch_counts_only_changed_coverable_lines() {
        let f = file(
            "a.rs",
            &[
                (1, Some(1)),
                (5, Some(0)),
                (12, Some(1)),
                (13, Some(2)),
                (15, None),
                (19, Some(1)),
            ],
        );
        let changed = BTreeSet::from([5, 12, 13, 15, 19, 40]);
        let s = Summary::of_patch(&f, &changed, true).unwrap();
        assert_eq!((s.lines, s.hits, s.misses, s.partials), (4, 1, 2, 1));
        assert_eq!(s.coverage, Some(25.0));
        assert_eq!(s.missed_lines.unwrap(), ["12", "19"]);
        assert_eq!(s.partial_lines.unwrap(), ["13"]);
    }

    #[test]
    fn a_patch_with_no_coverable_line_is_dropped() {
        let f = file("a.rs", &[(1, Some(1)), (2, None)]);
        assert!(Summary::of_patch(&f, &BTreeSet::from([2, 3]), true).is_none());
    }

    #[test]
    fn totals_sum_every_file_and_count_those_with_missing_lines() {
        let a = Summary::of(&file("a.rs", &[]), "", false);
        let mut b = a.clone();
        b.path = "b.rs".into();
        b.hits = 10;
        b.misses = 0;
        b.partials = 0;
        let t = Totals::of(&[a, b]);
        assert_eq!(
            (t.lines, t.hits, t.files, t.files_with_missing),
            (20, 15, 2, 1)
        );
        assert_eq!(t.coverage, Some(75.0));
        assert_eq!(Totals::of(&[]).coverage, None);
    }

    #[test]
    fn percentage_rounds_to_two_decimals() {
        assert_eq!(percentage(30, 44), Some(68.18));
        assert_eq!(percentage(2, 3), Some(66.67));
        assert_eq!(percentage(0, 0), None);
    }

    #[test]
    fn sort_by_misses_puts_the_largest_gap_first_and_ties_by_path() {
        let mk = |path: &str, misses, partials| Summary {
            path: path.into(),
            coverage: None,
            lines: 0,
            hits: 0,
            misses,
            partials,
            missed_lines: None,
            partial_lines: None,
        };
        let mut files = vec![mk("c", 1, 0), mk("b", 2, 1), mk("a", 1, 0)];
        sort(&mut files, SortKey::Misses);
        let order: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(order, ["b", "a", "c"]);
        sort(&mut files, SortKey::Path);
        let order: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(order, ["a", "b", "c"]);
    }

    #[test]
    fn null_fields_deserialize_to_defaults() {
        let f: FileReport = serde_json::from_str(
            r#"{"name":"a.rs","totals":null,"line_coverage":[[1,0],[2,null]]}"#,
        )
        .unwrap();
        assert_eq!(f.totals.lines, 0);
        assert_eq!(f.line_coverage, [(1, Some(0)), (2, None)]);
        let r: CoverageReport = serde_json::from_str(r#"{"totals":{},"files":null}"#).unwrap();
        assert_eq!(r.files.len(), 0);
    }
}
