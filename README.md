# codecov-coverage-cli
[![GitHub Release](https://img.shields.io/github/v/release/rwojsznis/codecov-coverage-cli)](https://github.com/rwojsznis/codecov-coverage-cli/releases/latest)
[![codecov](https://codecov.io/gh/rwojsznis/codecov-coverage-cli/graph/badge.svg)](https://codecov.io/gh/rwojsznis/codecov-coverage-cli)
[![CI](https://github.com/rwojsznis/codecov-coverage-cli/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/rwojsznis/codecov-coverage-cli/actions/workflows/ci.yml?query=branch%3Amain)

> [!NOTE]
> tldr: Ask Codecov which lines are not covered — for a file, for the whole repository, or for only the lines your branch changed. Made for coding agents: one binary, short output, no MCP server.

```bash
$ codecov-coverage-cli --diff origin/main
patch origin/main...d2b25d2d50: 37.5% of changed lines covered (3/8 hit), 3 of 4 changed files with missing coverage

app/models/order.rb: 25% (1/4 changed lines hit, 2 missed, 1 partial)
  missed:  12, 19
  partial: 13

lib/new.rb: 50% (1/2 changed lines hit, 1 missed, 0 partial)
  missed:  2
```

## Why?

An agent that writes tests needs to know which lines are still not covered. The Codecov UI shows that, but an agent cannot use it, and a raw API response is thousands of `[line, code]` pairs. This tool asks the Codecov API and prints only the gaps, as line ranges.

It started as an MCP server. A CLI is simpler: any agent that can run a shell command can use it, and you can use it yourself.

## Install

Get a binary from the [releases page](https://github.com/rwojsznis/codecov-coverage-cli/releases) (macOS arm64/x86_64, Linux x86_64 gnu/musl), or use [mise](https://mise.jdx.dev/dev-tools/backends/github.html):

```bash
mise use -g github:rwojsznis/codecov-coverage-cli
```

Or build it from source:

```bash
cargo install --locked --git https://github.com/rwojsznis/codecov-coverage-cli
```

## Configuration

| Variable | Flag | |
|---|---|---|
| `CODECOV_AUTH_TOKEN` | — | Codecov API token (required). Create one in Codecov under *Settings → Access*. |
| `CODECOV_SERVICE` | `--service` | `github`, `gitlab`, `bitbucket`, or their `_enterprise` / `_server` form |
| `CODECOV_OWNER` | `--owner` | user, organization or group (GitLab subgroups: `group/sub`) |
| `CODECOV_REPO` | `--repo` | repository name |
| `CODECOV_API_URL` | `--api-url` | API root for a self-hosted Codecov (default `https://api.codecov.io`) |

Inside a clone whose `origin` remote is on github.com, gitlab.com or bitbucket.org, the service, owner and repository come from the remote. Then only `CODECOV_AUTH_TOKEN` is necessary.

## Usage

Paths can be absolute or relative to the current directory. Inside a git repository they are resolved relative to its root, as Codecov expects.

### Files

```bash
$ codecov-coverage-cli src/index.ts src/codecov.ts --branch main
src/index.ts: 50% (5/10 lines hit, 3 missed, 2 partial)
  missed:  3-5, 9
  partial: 7, 10

src/codecov.ts: 100% (10/10 lines hit, 0 missed, 0 partial)
  fully covered
```

### All files with missing coverage

`--all` lists every file with missed or partial lines, from one request. Give directories to limit it.

```bash
$ codecov-coverage-cli --all src --files-only --sort misses -b my-feature
branch my-feature (src/): 67.5% overall (27/40 lines hit), 2 of 3 files with missing coverage

src/cli.ts: 60% (12/20 lines hit, 8 missed, 0 partial)
src/index.ts: 50% (5/10 lines hit, 3 missed, 2 partial)
```

### Patch coverage: only the lines a branch changed

`--diff <BASE>` runs `git diff <BASE>...<head>` locally and keeps only the added or modified lines that Codecov counts as coverable — the same question as Codecov's patch coverage.

The head commit is `HEAD`, or the commit that `--sha` / `--branch` names in the local repository (`--branch` falls back to `origin/<branch>`). The tool fetches the report for that exact commit, so the line numbers match the diff. CI must have uploaded coverage for it; if not, the tool tells you and exits with 1.

The percentage can differ a little from the Codecov pull request comment when the pull request base is not the merge base with `<BASE>`.

### Options

| | |
|---|---|
| `-a`, `--all` | all files with missing coverage; arguments are directories |
| `-d`, `--diff <BASE>` | only lines changed since `<BASE>`; arguments are directories |
| `-l`, `--files-only` | per-file totals, no line ranges |
| `--sort path\|misses` | order of files for `--all` and `--diff` (default `path`) |
| `-b`, `--branch <NAME>` | report for the head of a branch (default: the default branch) |
| `-s`, `--sha <SHA>` | report for one commit |
| `--json` | JSON: an array of files; `{totals, files}` for `--all`; `{base, head, totals, files}` for `--diff` |
| `--raw` | Codecov's reports as a JSON array (not with `--diff`) |

### Exit codes

`0` success (also when coverage is missing), `1` a report could not be fetched, `2` usage, configuration or git error.

## For your agent

Add something like this to your `AGENTS.md` / `CLAUDE.md`:

```markdown
To find lines without test coverage, run `codecov-coverage-cli --diff origin/main`
(changed lines only) or `codecov-coverage-cli <file>...` (whole files).
Coverage comes from the last CI run for the commit, so push and wait for CI first.
```

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo llvm-cov --all-targets   # coverage
```

The tests do not use the network: they run the binary against a mock Codecov API and real temporary git repositories. See [AGENTS.md](AGENTS.md).

## License

MIT, see [LICENSE.txt](LICENSE.txt).
