# ghelpr

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
ghelpr comments <pr> [--owner <owner>] [--repo <repo>] [--all] [--full]
```

Returns review threads as JSON. By default, only unresolved threads are shown with a summary of each comment.

- `--all` - include resolved threads
- `--full` - include full details (diff hunks, review state, line positions, etc.)

Owner and repo are inferred from the current git remote by default.

Auth is resolved from `GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token` (in that order).
