# `rai git` / `rai gh` / `rai gwq` / `rai dev`

リポジトリ操作系の小さなサブコマンド群。

## `rai git`

仕様: `docs/specs/06-git-autopull.md` / `docs/specs/07-git-track-mine.md`。

| Command | 用途 |
|---|---|
| `rai git autopull` | upstream を一定間隔で fetch し、HEAD と乖離していたら **fast-forward だけで** pull する |
| `rai git track-mine` | 自分が author の最近のブランチを一覧し、選択して checkout (tracking branch を生やす) |

### `git autopull`

長時間張り付いている開発ブランチで「気付かないうちに upstream が進んでた」を潰すための間欠 fetcher。fast-forward 不可なら何もしない (rebase / merge は走らない)。

### `git track-mine`

レビュー対応 / rebase の起点として、いちいち `git fetch && git checkout -t origin/...` を打たなくて済むようにする。fzf で 1 つ選んで checkout する。

## `rai gh`

仕様: `docs/specs/03-gh-rate-limit.md`。

| Command | 用途 |
|---|---|
| `rai gh rate-limit` | GitHub API のレートリミット残量と reset までの残り時間を人間に読める形式で表示 |

fish 関数 `gh_rate_limit` の置き換え。

## `rai gwq`

仕様: `docs/specs/10-gwq-clean.md`。

| Command | 用途 |
|---|---|
| `rai gwq clean` | `gwq` で増えた worktree のうち「もう要らないもの」を fzf で選んでまとめて掃除する |

「もう要らない」の判定基準: default branch にマージ済み / リモートが消えた / 古い。fish 関数 `gwq-clean` (248 行) の Rust 移植。

## `rai dev`

仕様: `docs/specs/05-dev.md`。

`ghq` + `gwq` で管理しているリポジトリ / worktree から fzf で 1 つを選び、選択結果のパスを stdout に出す。

**注意**: bin から親シェルの cwd は変えられないので、選択結果を吐くだけにとどめ、`cd` / `tmux rename` はシェル側 wrapper の責務。fish なら例えば:

```fish
function dev
    set -l path (rai dev) or return
    cd $path
end
```

## See Also

- [cmd-issue-pr](wiki://cmd-issue-pr) — `gh` を使う他の系列
- [cmd-develop](wiki://cmd-develop) — `gwq` を使う側の最大ユーザー
- [shell-execution-policy](wiki://shell-execution-policy) — `gh` / `git` / `gwq` 呼び出しの規約
