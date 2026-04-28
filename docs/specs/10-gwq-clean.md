# 10 — `rai gwq clean`

Source issue: [#10](https://github.com/masseater/rai/issues/10)

## 目的

`gwq` で増えた worktree のうち「もう要らないもの (default branch にマージ済み / リモートが消えた / 古い)」を fzf で選んでまとめて掃除する。fish 関数 `gwq-clean` (248 行) の Rust 移植。

## 機能要件

- default branch 検出: `git symbolic-ref refs/remotes/origin/HEAD` → fallback `refs/heads/main` → `refs/heads/master`。どれも無ければ exit 1。
- `git fetch --prune --quiet` を先に走らせる。
- worktree 列挙 (`git worktree list --porcelain`):
  - bare と default branch をスキップ。
  - 各 worktree について以下を判定:
    - `MERGED`: `git branch --merged origin/<default>` に含まれる。
    - `GONE`: `git branch -vv` で `: gone]` がついている。
    - `DIRTY`: `git -C <path> status --porcelain` が非空。
    - `ACTIVE`: 上記いずれでもない。
  - 表示: `[STATUS]  branch  last_commit  clean|dirty`。
- fzf:
  - MERGED / GONE は **preselect** された状態で起動する。
  - `--include-dirty` がない限り、DIRTY は preselect しない。
  - `--multi --reverse --no-sort --ansi`、cancel で exit 0。
- 確認プロンプト: `よろしいですか？ [y/N]` (fish 版と一字一句揃える)。`--yes` でスキップ。
- 削除手順 (1 worktree あたり):
  1. `gwq remove -f -b <branch>`。
  2. ディレクトリが残っていれば `rm -rf <wt_path>` + `git worktree prune`。
  3. ブランチが残っていれば (squash-merged 等) `git branch -D <branch>`。
  4. 検証: `refs/heads/<branch>` がまだあれば `✗ failed`、消えていれば `✓`。
- フラグ: `--all`、`--include-dirty`、`--default-branch BR`、`--remote NAME`、`--dry-run`、`--yes`、`--json`。
- non-tty では `--json` 必須 (誤動作防止)。
- fzf 起動中の SIGINT で raw mode が確実に戻る。

## 受け入れ条件

- [ ] default branch 検出が origin/HEAD → main → master の順で動く。
- [ ] MERGED / GONE が preselect される。
- [ ] DIRTY 表示が現行どおり (`--include-dirty` なしでは preselect されない)。
- [ ] gwq remove 失敗時に `rm -rf` + `git worktree prune` の fallback が動く。
- [ ] squash-merged で残るブランチを `git branch -D` する fallback が動く。
- [ ] 削除後に refs/heads/<br> が残っていれば `✗` が表示される。
- [ ] `--dry-run` で副作用ゼロ。
- [ ] submodule worktree (rm -rf fallback 経路) が壊れない。

## 期待する成果物

- `crates/cmd/rai-cmd-gwq` crate。
- `rai gwq clean` を本体に配線。
- README に fish からの移行手順 (`alias gwq-clean 'rai gwq clean'`) を記載。

## 非対象

- worktree の add/move (gwq 本体の機能)。
