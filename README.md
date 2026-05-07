# ghelpr

[![Publish](https://github.com/lkurcak/ghelpr/workflows/Publish/badge.svg)](https://github.com/lkurcak/ghelpr/actions)
[![Crates.io](https://img.shields.io/crates/v/ghelpr.svg)](https://crates.io/crates/ghelpr)

CLI tool for working with GitHub pull requests.

## Install

```
brew install lkurcak/tap/ghelpr
```

or

```
cargo install ghelpr
```

or download a prebuilt binary from [GitHub Releases](https://github.com/lkurcak/ghelpr/releases).

## Usage

```
ghelpr [-o <owner>] [-r <repo>] comments <pr> [-a] [-f]
```

Returns review threads as JSON. By default, only unresolved threads are shown with a summary of each comment.

- `-a`, `--all` - include resolved threads
- `-f`, `--full` - include full details (diff hunks, review state, line positions, etc.)
- `-o`, `--owner` - repository owner (inferred from git remote if omitted)
- `-r`, `--repo` - repository name (inferred from git remote if omitted)

Auth is resolved from `GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token` (in that order).
