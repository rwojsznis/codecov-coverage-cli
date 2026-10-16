# AGENTS.md

You must use ASD-STE100 Simplified Technical English (STE) when it doesn't detract from meaning.

## What this is

`codecov-coverage-cli` prints the lines that Codecov reports as not covered.
It is made for coding agents: the output is short, stable and easy to parse.
Single crate, one binary (`codecov-coverage-cli`) plus a library
(`codecov_coverage_cli`).

It has three modes:

- **files** (default): one `file_report` request for each path.
- **`--all`**: one `report` request (one for each directory prefix), then only
  the files with missed or partial lines.
- **`--diff <BASE>`**: patch coverage. `git diff BASE...head` gives the changed
  lines; the `report` for that exact head commit gives their coverage.

## Layout

| Path | What |
|---|---|
| `src/api.rs` | the HTTP client: the two Codecov API v2 endpoints, URL encoding |
| `src/report.rs` | report types, `Summary`, `Totals`, line ranges, sort |
| `src/diff.rs` | `git diff -U0` parser, C-quoted paths, common directory prefix |
| `src/git.rs` | the `git` binary: root, commits, the diff, the `origin` remote |
| `src/paths.rs` | command-line paths to repository-relative paths |
| `src/cli/mod.rs` | clap only: flags, configuration, exit codes, dispatch |
| `src/cli/modes.rs` | the three modes and the text and JSON output |
| `src/cli/out.rs` | `outln!` / `errln!`: a closed pipe is not a panic |
| `src/main.rs` | the entry point: run, print the failure, pick the exit code |
| `tests/` | integration tests: the binary against a mock API and real git repositories |

Unit tests live in `#[cfg(test)] mod tests` next to the code. CLI-level tests
live in `tests/`. `tests/common/mod.rs` has the mock Codecov server (a
`TcpListener` on a thread, no extra crates) and the temporary git repository.

## Workflow — do it in this order

1. **Write the failing test first.** Before any implementation, add a test that
   fails for the reason you expect. Run it and *read the failure*.
2. **Then write the implementation**, and only enough of it to make that test
   pass.
3. **Check your work.** Run the full suite, not just the new test.
4. **Run the linters last**, and get them clean before you call the work done.

```bash
cargo test --quiet                           # full suite; show details only on failure
cargo clippy --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items
cargo fmt
```

Keep successful test output brief. If the quiet test run fails, inspect and
report the complete failure output.

Do not report a change as finished until every one of them is green. If
something is left failing or unfinished, say so explicitly rather than narrowing
the scope quietly.

## Conventions

- Exit codes: **0** success, **1** a report could not be fetched, **2** usage,
  configuration or git error. A "missing coverage" result is still 0.
- The text output is an interface: agents read it. Change it only on purpose,
  and update the exact-output assertions in `tests/` in the same commit.
- No panics in shipped code (`clippy::unwrap_used`, `expect_used`, `panic`).
  Print through `outln!` / `errln!`, never `println!`.
- Tests never use the network or the developer's git configuration:
  `tests/common` points the binary at the mock API and sets
  `GIT_CONFIG_GLOBAL=/dev/null`.
- Write in-code comments that describe why code does what it does, not what it
  does. Be concise and direct.

## Gotchas

- `line_coverage` codes: `0` hit, `1` miss, `2` partial, `null` not coverable.
  The report endpoint documents them; the file report endpoint does not.
- Codecov keys files by repository-relative path. A path through a symlink
  (`/var` → `/private/var` on macOS) must resolve to the physical root that
  `git rev-parse --show-toplevel` gives, also for a file that does not exist.
- `--diff` fetches coverage for one exact SHA so that the diff and the report
  agree on line numbers. A commit without an upload is a 404, not a fallback.
- `git diff` runs with every format option pinned (`--src-prefix`,
  `--no-ext-diff`, `core.quotePath=false`, …); a user's `diff.noprefix` must not
  break the parse.
- A whole-repository report can be larger than `ureq`'s default 10 MB body
  limit; `api.rs` raises it.
