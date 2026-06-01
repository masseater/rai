# 24 — `rai pr watch-loop`

複数の GitHub PR を 1 つの親 watcher で監視し、PR に修正が必要な更新が入った時だけ
既存の `rai develop pr` を起動して agent に対応を委ねる。

## Purpose

個別セッションごとに GitHub API を polling すると rate limit を消費しすぎるため、PR
監視は単一の親プロセスに集約する。ユーザーは watcher を基本的に background で起動し、
あとから TUI で稼働中 watcher の状態確認と停止を行える。

## CLI

```sh
rai pr watch-loop
rai pr watch-loop start [OPTIONS] [PR]...
rai pr watch-loop tui
rai pr watch-loop status
rai pr watch-loop stop <ID>
```

- サブコマンドなしの `rai pr watch-loop` は `rai pr watch-loop tui` と同じ。
- `<PR>` は PR URL または番号。番号は `--repo OWNER/REPO` と組み合わせる。
- `<PR>` 省略時は `gh` のログインユーザーを取得し、対象 repo にある自分の open PR を
  選択 UI で表示する。
- 対象 repo は、git repository 内では現在の GitHub repository を使う。git repository
  外では `OWNER/REPO` を対話入力してもらう。非TTYでは `--repo` を必須にする。
- `start` は既定で daemon 化して即座に戻る。`--foreground` で前面実行できる。
- `--interval SECS` で polling 間隔を指定する。
- `--trigger-initial` を指定した場合、初回取得時点で修正対象の PR も agent 起動対象にする。
- `--on-any-update` を指定した場合、CI 失敗や conflict に限らず PR fingerprint 変化で agent
  を起動する。
- `--engine-cmd` / `--prompt-template` / `--permission-mode` / `--no-auto-publish` は
  `rai develop pr` に渡す。

## Watch Behavior

- 親 watcher は repo ごとに対象 PR をまとめ、1 repo につき 1 回の GraphQL query で
  `headRefOid`, `mergeable`, `reviewDecision`, `updatedAt`, `statusCheckRollup` を取得する。
- PR ごとに fingerprint を作り、前回 fingerprint と差分がある時だけ処理候補にする。
- 既定で agent を起動するのは、差分後の PR が以下のいずれかに該当する時:
  - mergeable が `CONFLICTING`
  - check / status に failure 系状態がある
  - reviewDecision が `CHANGES_REQUESTED`
- agent 起動は `rai develop pr <PR_URL>` に委譲する。watch-loop 自体は worktree や修正内容を
  直接扱わない。
- 同一 fingerprint に対しては 1 回だけ起動する。

## State / TUI

- watcher は state directory に JSON state を定期保存する。
- TUI は state file を読み、watcher ID、pid、対象 PR、最終 poll、最終 agent 起動、最終エラーを表示する。
- TUI から選択中 watcher を停止できる。
- TUI から新しい watcher を起動できる。repo 入力、自分の open PR 一覧、複数選択、
  watcher 起動は TUI 内で完結し、外部の fzf や別画面へ遷移しない。
- TUI の PR 一覧では、PR ごとに `rai develop pr` へ渡す `--engine-cmd`、
  `--prompt-template`、`--permission-mode`、`--no-auto-publish` を設定できる。
  設定は watcher state に PR 単位で保存され、該当 PR の agent 起動時だけ適用される。
- TUI の watcher 追加は、現在の GitHub repository を非同期に解決し、解決できた値を
  `OWNER/REPO` の初期入力として使う。解決待ち中も入力欄は即座に操作でき、ユーザー入力を
  後から上書きしない。
- stale な pid は status/TUI 上で stopped として表示する。

## Non-goals

- agent 出力の解析。
- tmux session の完全な lifecycle 管理。
- GitHub webhook server の常駐。
