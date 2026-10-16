//! Command-line paths to the repository-relative form Codecov keys files by.

use std::path::{Component, Path, PathBuf};

/// `arg` relative to the repository root, with `/` separators.
///
/// A relative `arg` is taken from `cwd`, so `a.rs` inside `src/` is
/// `src/a.rs`. A path outside the repository is returned as given; Codecov
/// then answers that it has no such file. Outside a repository there is no
/// root to resolve against, so only a leading `./` goes.
pub fn to_repo_path(arg: &str, cwd: &Path, root: Option<&Path>) -> String {
    let Some(root) = root else {
        return arg.strip_prefix("./").unwrap_or(arg).to_owned();
    };
    let absolute = normalize(&cwd.join(arg));
    if let Some(relative) = relative_to(&absolute, root) {
        return relative;
    }
    // `cwd` or `arg` may go through a symlink (`/var` → `/private/var` on
    // macOS) while git reports the physical root. Tried second, so a symlink
    // *inside* the repository keeps the name it has there.
    physical(&absolute)
        .and_then(|physical| relative_to(&physical, root))
        .unwrap_or_else(|| arg.to_owned())
}

/// A path prefix for the report endpoint. An existing directory gets a
/// trailing `/` so `src` does not also match `src2/`. The repository root
/// itself is `""`: no filter.
pub fn to_repo_prefix(arg: &str, cwd: &Path, root: Option<&Path>) -> String {
    let prefix = to_repo_path(arg, cwd, root);
    if prefix.is_empty() || prefix == "." {
        return String::new();
    }
    if cwd.join(arg).is_dir() && !prefix.ends_with('/') {
        format!("{prefix}/")
    } else {
        prefix
    }
}

fn relative_to(path: &Path, root: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    Some(parts.join("/"))
}

/// `path` with its deepest existing ancestor resolved through any symlinks.
/// The rest is appended as written: a file deleted on the branch, or one that
/// was never checked out, still has a name.
fn physical(path: &Path) -> Option<PathBuf> {
    let mut existing = path;
    let mut missing = Vec::new();
    loop {
        if let Ok(mut resolved) = std::fs::canonicalize(existing) {
            resolved.extend(missing.iter().rev());
            return Some(resolved);
        }
        missing.push(existing.file_name()?);
        existing = existing.parent()?;
    }
}

/// Resolves `.` and `..` without touching the disk, so a deleted file still
/// has a name.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_is_taken_from_the_working_directory() {
        let root = Path::new("/repo");
        let cwd = Path::new("/repo/src");
        assert_eq!(to_repo_path("a.rs", cwd, Some(root)), "src/a.rs");
        assert_eq!(to_repo_path("./a.rs", cwd, Some(root)), "src/a.rs");
        assert_eq!(to_repo_path("../lib/b.rs", cwd, Some(root)), "lib/b.rs");
        assert_eq!(to_repo_path("/repo/lib/b.rs", cwd, Some(root)), "lib/b.rs");
    }

    #[test]
    fn a_path_outside_the_repository_is_kept_as_given() {
        let root = Path::new("/repo");
        assert_eq!(
            to_repo_path("../../etc/x", Path::new("/repo/src"), Some(root)),
            "../../etc/x"
        );
    }

    #[test]
    fn without_a_repository_only_a_leading_dot_slash_goes() {
        assert_eq!(
            to_repo_path("./src/a.rs", Path::new("/x"), None),
            "src/a.rs"
        );
        assert_eq!(to_repo_path("src/a.rs", Path::new("/x"), None), "src/a.rs");
    }

    #[cfg(unix)]
    #[test]
    fn a_path_through_a_symlink_resolves_even_when_the_file_does_not_exist() {
        let dir = std::env::temp_dir().join("codecov-coverage-cli-paths-symlink");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("real/src")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
        let root = std::fs::canonicalize(dir.join("real")).unwrap();
        let cwd = dir.join("link/src");
        assert_eq!(to_repo_path("gone.rs", &cwd, Some(&root)), "src/gone.rs");
        assert_eq!(
            to_repo_path(
                dir.join("link/new/dir/a.rs").to_str().unwrap(),
                &cwd,
                Some(&root)
            ),
            "new/dir/a.rs"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_repository_root_as_a_prefix_is_no_filter() {
        let root = Path::new("/repo");
        assert_eq!(to_repo_prefix(".", Path::new("/repo"), Some(root)), "");
        assert_eq!(to_repo_prefix("..", Path::new("/repo/src"), Some(root)), "");
    }

    #[test]
    fn an_existing_directory_prefix_gets_a_trailing_slash() {
        let dir = std::env::temp_dir().join("codecov-coverage-cli-paths-prefix");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        assert_eq!(to_repo_prefix("src", &dir, Some(&dir)), "src/");
        assert_eq!(to_repo_prefix("src/", &dir, Some(&dir)), "src/");
        // Not a directory on disk: a plain prefix, as given.
        assert_eq!(to_repo_prefix("lib/a", &dir, Some(&dir)), "lib/a");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
