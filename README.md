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
| `rai issue`      | Inventory Issues with an agent and triage results.                |
| `rai develop`    | Develop from an Issue or rescue an existing PR (conflict / CI fix).|
| `rai gwq`        | gwq worktree cleanup helpers.                                     |
| `rai conflicts`  | Long-running batch that resolves CONFLICTING PRs via an agent.    |
| `rai completion` | Emit a shell completion script (bash / zsh / fish / powershell / elvish). |

Run `rai <subcommand> --help` for details on each.

### Develop / rescue workflow

Spawn an agent in a dedicated worktree + tmux session for either a fresh Issue
or an in-flight PR:

```sh
# Develop a GitHub Issue end-to-end (worktree, tmux, agent, finalize → PR).
rai develop issue <ISSUE_URL_OR_NUMBER>

# Rescue an existing PR (resolve conflicts and/or fix failing CI).
rai develop pr <PR_URL_OR_NUMBER>
```

If you previously used the fish function for the issue flow, point it at the
new subcommand:

```fish
alias gh-issue-fix 'rai develop issue'
```

To inventory Issues without letting the AI fetch them, have `rai` collect the
Issue JSON, pass the fixed prompt to your engine, and persist the verdict on
each Issue as a comment + `triage:*` label so you can mechanically process the
results without re-asking the AI:

```sh
# Dry-run preview only.
rai issue inventory --repo OWNER/REPO --engine-cmd "ccs_print c1"

# Commit comments and labels to GitHub.
rai issue inventory --repo OWNER/REPO --engine-cmd "ccs_print c1" --apply

# Save engine output and re-apply later without rerunning the AI.
rai issue inventory --save-verdicts /tmp/v.txt
rai issue inventory --from-verdicts /tmp/v.txt --apply

# Review the labeled issues one by one and apply close/keep decisions.
# Shows body + comments for each issue and prompts c/k/s/q. The actual
# `gh issue close` and label removal are batched after the loop.
rai issue triage --repo OWNER/REPO --reason completed
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
