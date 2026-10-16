//! The added and modified lines of a `git diff -U0`, by file.

use std::collections::{BTreeMap, BTreeSet};

/// Line numbers in the *new* version of each file. A file that only lost
/// lines, a deleted file and a binary file are absent.
pub type ChangedLines = BTreeMap<String, BTreeSet<u64>>;

/// Reads the output of `git diff -U0 --src-prefix=a/ --dst-prefix=b/`.
///
/// Only the `+++` header and the `@@` hunk headers matter: with zero context
/// lines, a hunk's new-side range is exactly the lines it added or replaced.
pub fn parse(diff: &str) -> ChangedLines {
    let mut changes = ChangedLines::new();
    let mut current: Option<&mut BTreeSet<u64>> = None;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            // `/dev/null` (a deletion) has no `b/` prefix, so it ends up with no set.
            current = unquote(path)
                .strip_prefix("b/")
                .map(|path| changes.entry(path.to_owned()).or_default());
            continue;
        }
        if let (Some(lines), Some((start, count))) = (current.as_deref_mut(), new_range(line)) {
            lines.extend(start..start + count);
        }
    }
    changes.retain(|_, lines| !lines.is_empty());
    changes
}

/// `@@ -12,2 +14,3 @@ fn x` → `(14, 3)`. A missing count means 1; a count of
/// 0 is a pure deletion.
fn new_range(line: &str) -> Option<(u64, u64)> {
    let rest = line.strip_prefix("@@ -")?;
    let (_, rest) = rest.split_once(" +")?;
    let (range, _) = rest.split_once(" @@")?;
    match range.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}

/// Undoes git's C-style quoting of a path with unusual bytes, e.g.
/// `"b/caf\303\251.rb"`. `core.quotePath=false` keeps UTF-8 as is, but a
/// quote, a backslash or a control character is still quoted.
pub fn unquote(path: &str) -> String {
    let Some(inner) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
        return path.to_owned();
    };
    let raw = inner.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] != b'\\' || i + 1 == raw.len() {
            bytes.push(raw[i]);
            i += 1;
            continue;
        }
        let octal = raw
            .get(i + 1..i + 4)
            .filter(|digits| digits.iter().all(|d| (b'0'..=b'7').contains(d)));
        if let Some(digits) = octal {
            let value = digits
                .iter()
                .fold(0u32, |acc, d| acc * 8 + u32::from(d - b'0'));
            bytes.push(u8::try_from(value).unwrap_or(b'?'));
            i += 4;
            continue;
        }
        bytes.push(match raw[i + 1] {
            b'a' => 0x07,
            b'b' => 0x08,
            b'f' => 0x0C,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 0x0B,
            other => other,
        });
        i += 2;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The deepest directory (with a trailing `/`) that holds every path, or `""`.
/// The report endpoint filters by prefix, so asking for only that part of the
/// tree keeps a large repository's report small.
pub fn common_dir_prefix<'a>(paths: impl IntoIterator<Item = &'a str>) -> String {
    let mut paths = paths.into_iter();
    let Some(first) = paths.next() else {
        return String::new();
    };
    let mut prefix = &first[..first.rfind('/').map_or(0, |i| i + 1)];
    for path in paths {
        while !path.starts_with(prefix) {
            // Drop the last directory: "a/b/" → "a/".
            let trimmed = &prefix[..prefix.len() - 1];
            prefix = &prefix[..trimmed.rfind('/').map_or(0, |i| i + 1)];
        }
    }
    prefix.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/app/models/old_name.rb b/app/models/new_name.rb
similarity index 77%
rename from app/models/old_name.rb
rename to app/models/new_name.rb
--- a/app/models/old_name.rb
+++ b/app/models/new_name.rb
@@ -3 +3 @@ x 2
-x 3
+x 3 edited
diff --git a/app/models/order.rb b/app/models/order.rb
--- a/app/models/order.rb
+++ b/app/models/order.rb
@@ -5 +5 @@ line 4
-line 5
+line 5 changed
@@ -12,2 +12,2 @@ line 11
-line 12
-line 13
+line 12 changed
+line 13 changed
@@ -18,0 +19 @@ line 18
+inserted
@@ -20,2 +20,0 @@ line 19
-gone 1
-gone 2
diff --git a/lib/gone.rb b/lib/gone.rb
deleted file mode 100644
--- a/lib/gone.rb
+++ /dev/null
@@ -1,2 +0,0 @@
-1
-2
diff --git a/only_deletes.rb b/only_deletes.rb
--- a/only_deletes.rb
+++ b/only_deletes.rb
@@ -4 +3,0 @@
-x
diff --git \"a/lib/caf\\303\\251.rb\" \"b/lib/caf\\303\\251.rb\"
new file mode 100644
--- /dev/null
+++ \"b/lib/caf\\303\\251.rb\"
@@ -0,0 +1,2 @@
+u1
+u2
Binary files a/logo.png and b/logo.png differ
";

    #[test]
    fn parse_keeps_added_and_modified_lines_of_the_new_side() {
        let changes = parse(DIFF);
        let lines = |path: &str| changes[path].iter().copied().collect::<Vec<_>>();
        assert_eq!(lines("app/models/order.rb"), [5, 12, 13, 19]);
        assert_eq!(lines("app/models/new_name.rb"), [3]);
        assert_eq!(lines("lib/café.rb"), [1, 2]);
    }

    #[test]
    fn parse_drops_deletions_and_binary_files() {
        let changes = parse(DIFF);
        assert!(!changes.contains_key("lib/gone.rb"));
        assert!(!changes.contains_key("only_deletes.rb"));
        assert!(!changes.contains_key("logo.png"));
        assert_eq!(changes.len(), 3);
    }

    #[test]
    fn a_hunk_header_without_a_count_is_one_line() {
        assert_eq!(new_range("@@ -3 +7 @@"), Some((7, 1)));
        assert_eq!(new_range("@@ -3,2 +7,0 @@ ctx"), Some((7, 0)));
        assert_eq!(new_range("@@@ -1 -1 +1 @@@"), None);
        assert_eq!(new_range("+@@ -1 +1 @@"), None);
    }

    #[test]
    fn unquote_decodes_octal_bytes_and_escapes() {
        assert_eq!(unquote(r#""b/caf\303\251.rb""#), "b/café.rb");
        assert_eq!(unquote(r#""b/a\"q\\b\tc""#), "b/a\"q\\b\tc");
        assert_eq!(unquote("b/plain.rb"), "b/plain.rb");
    }

    #[test]
    fn common_dir_prefix_is_the_deepest_shared_directory() {
        assert_eq!(
            common_dir_prefix(["app/models/a.rb", "app/models/b.rb"]),
            "app/models/"
        );
        assert_eq!(
            common_dir_prefix(["app/models/a.rb", "app/views/b.erb"]),
            "app/"
        );
        assert_eq!(common_dir_prefix(["app/a.rb", "lib/b.rb"]), "");
        assert_eq!(common_dir_prefix(["app/a.rb", "app2/b.rb"]), "");
        assert_eq!(common_dir_prefix(["README.md"]), "");
        assert_eq!(common_dir_prefix([]), "");
    }
}
