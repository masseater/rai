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
| `rai issue`      | Develop from GitHub Issues and inventory issue lists with an agent. |
| `rai gwq`        | gwq worktree cleanup helpers.                                     |
| `rai conflicts`  | Long-running batch that resolves CONFLICTING PRs via an agent.    |
| `rai completion` | Emit a shell completion script (bash / zsh / fish / powershell / elvish). |

Run `rai <subcommand> --help` for details on each.

### Migrating issue workflow

If you previously used the fish function, point it at the `develop` subcommand:

```fish
alias gh-issue-fix 'rai issue develop'
```

To inventory Issues without letting the AI fetch them, have `rai` collect the
Issue JSON and pass the fixed prompt to your engine:

```sh
rai issue inventory --repo OWNER/REPO --engine-cmd "ccs_print c1"
```

## Shell Completion

`rai completion <shell>` writes a completion script to stdout. The definition
is generated from the binary's clap command tree.

For persistent setup, prefer `--source`: it prints a small rc/config snippet
that reloads completions from the current `rai` binary when a new shell starts,
so completions follow upgrades and newly added subcommands automatically.

```sh
# fish
rai completion fish | source
# Persist in ~/.config/fish/config.fish:
rai completion --source fish >> ~/.config/fish/config.fish

# zsh (append the output to ~/.zshrc)
rai completion --source zsh >> ~/.zshrc

# bash
source <(rai completion bash)
# Persist in ~/.bashrc:
rai completion --source bash >> ~/.bashrc
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
