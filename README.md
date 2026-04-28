# rai

An extensible personal CLI for the author's day-to-day workflow.

## Overview

`rai` ships as a single binary that dispatches to subcommands wrapping tools the
author uses constantly (git, gh, gwq, claude, etc.). The repository is a Cargo
workspace where each `rai <subcommand>` lives in its own library crate, so new
commands can be added by editing one enum variant and one match arm.

## Installation

```sh
brew tap masseater/rai
brew install rai
```

Or build from source:

```sh
cargo install --path crates/rai
```

## Subcommands

| Command          | Description                                                       |
| ---------------- | ----------------------------------------------------------------- |
| `rai hello`      | Sample subcommand and template for new commands.                  |
| `rai pair`       | Run two commands in alternation with a fixed status bar.          |
| `rai date`       | Thin date formatter compatible with the fish `mydate` function.   |
| `rai gh`         | GitHub CLI (`gh`) helpers.                                        |
| `rai claude`     | Claude Code (`claude` CLI) helpers.                               |
| `rai dev`        | Pick a repo / worktree via `ghq` + `gwq` + `fzf`.                 |
| `rai git`        | Git utility subcommands (autopull, track-mine, …).                |
| `rai pr`         | GitHub Pull Request helpers.                                      |
| `rai issue`      | Spin up worktree + tmux + agent from a GitHub Issue.              |
| `rai gwq`        | gwq worktree cleanup helpers.                                     |
| `rai conflicts`  | Long-running batch that resolves CONFLICTING PRs via an agent.    |
| `rai completion` | Emit a shell completion script (bash / zsh / fish / powershell / elvish). |

Run `rai <subcommand> --help` for details on each.

### Migrating issue workflow

If you previously used the fish function, point it at the `develop` subcommand:

```fish
alias gh-issue-fix 'rai issue develop'
```

## Shell Completion

`rai completion <shell>` writes a completion script to stdout. The definition
is generated from the binary's clap command tree, so adding a subcommand and
rebuilding is enough to refresh the completions.

```sh
# fish
rai completion fish | source
# Persist:
rai completion fish > ~/.config/fish/completions/rai.fish

# zsh (drop into a directory on $fpath)
rai completion zsh > "${fpath[1]}/_rai"

# bash
rai completion bash > ~/.local/share/bash-completion/completions/rai
# Or load on the fly:
source <(rai completion bash)
```

## Development Commands

| Command                                                 | Description                          |
| ------------------------------------------------------- | ------------------------------------ |
| `cargo fmt --all`                                       | Format the workspace                 |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint with warnings promoted to errors |
| `cargo test --workspace`                                | Run all tests                        |
| `cargo run -p rai -- <subcommand>`                      | Run a subcommand locally             |

> Development commands last updated: 2026-04-28

For details on layout, conventions, and how to add a subcommand, see
[AGENTS.md](AGENTS.md).

## Related Links

- Specs: [`docs/specs/`](docs/specs/)
- Homebrew tap: [`masseater/homebrew-rai`](https://github.com/masseater/homebrew-rai)

## License

MIT OR Apache-2.0
