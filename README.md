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

## Usage

```
ghelpr unresolved <pr> [--owner <owner>] [--repo <repo>] [--include-outdated]
```

Returns unresolved review threads as JSON. Owner and repo are inferred from the current git remote by default.

Auth is resolved from `GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth token` (in that order).
