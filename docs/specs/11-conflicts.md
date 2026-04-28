# 11 — `rai conflicts`

Source issue: [#11](https://github.com/masseater/rai/issues/11)

## 目的

CONFLICTING になっている自分の open PR を検出し、専用 worktree で agent CLI を呼んでコンフリクトを自動解消し、`git push --force-with-lease` まで完走させる長時間バッチ。fish 関数 `resolve-conflicts` (533 行) の Rust 移植。本リポジトリ最大の負債。

## 機能要件

### サブコマンド構成

```
rai conflicts run --agent-cmd "<cmd>"
                  [--author @me] [--all] [--interval 300] [--jobs 3]
                  [--once] [--state-dir PATH] [--cache-dir PATH]
                  [PR ...]
rai conflicts status        [--state-dir PATH]
rai conflicts reset-failed  [--state-dir PATH]
```

### 状態保存場所 (既定)

- `~/.local/state/resolve-conflicts/queue.json`
- `~/.local/state/resolve-conflicts/logs/<pr>-<sha>.log`
- `~/.local/state/resolve-conflicts/queue.lock` (排他)
- `~/.cache/resolve-conflicts/wt/<pr>` (作業 worktree)

queue.json のスキーマは fish 版と互換 (移行不要):

```jsonc
{
  "version": 1,
  "updated_at": "...Z",
  "entries": {
    "<pr>": {
      "head_sha": "...", "base_ref": "...", "head_ref": "...",
      "mergeable": "CONFLICTING",
      "title": "...", "url": "...",
      "status": "pending|claimed|processing|done|failed",
      "attempts": 0,
      "enqueued_at": "...Z", "updated_at": "...Z",
      "started_at": "...Z", "finished_at": "...Z",
      "log_path": "...", "error": "..."
    }
  }
}
```

### main loop の振る舞い

- enqueue:
  - explicit PR 引数があればそれだけを enqueue (CONFLICTING フィルタを **バイパス**, 強制 retry)。
  - なければ `gh pr list --state open [--author @me] [--all]` の中から `mergeable == CONFLICTING` のみ。
  - 新規 entry / `head_sha` 変化があったら `status=pending` に上書き。
- worker (`--jobs` 上限):
  - 最古の pending を 1 つ pop して `claimed` に書き換え (lock 内)。
  - 1 worker = 1 PR を処理。
- reap:
  - 終わった worker を回収。
- 終了条件:
  - `--once`: pending=0 かつ alive=0 になったら break。
  - SIGINT / SIGTERM: 新規 spawn を止め、in-flight worker は **kill せず最後まで待つ**。
- shutdown 時に `status` を tsv で表示。

### 1 worker の処理 (1 PR)

1. ログを `<state>/logs/<pr>-<sha>.log` に append-only で開く。
2. 既存 worktree を削除 (`git worktree remove --force <wt>`) して stale を掃除。
3. `git fetch origin pull/<pr>/head:refs/remotes/origin/pr/<pr> --force` と base ref の fetch。
4. `git worktree add --detach <wt> origin/pr/<pr>` → `gh pr checkout <pr> --force`。
5. `git merge --no-edit origin/<base>`:
   - clean 経路: `ahead > 0` なら `git push --force-with-lease`。
   - conflict 経路:
     - prompt = 「PR #<n> (title) のコンフリクトを解消して `git push --force-with-lease` まで完了させてください…」
     - agent_cmd を **`shell-words::split` で分解** して `Command` を組む (eval せず、shell injection を防ぐ)。
     - agent exit != 0: `git merge --abort` → failed (`agent exit=…`)。
     - 未解消マーカが残る: `git merge --abort` → failed (`unresolved markers remain`)。
     - dirty: `git add -A && git commit -m "Merge <base> into PR #<n> (automated conflict resolution)"`。
     - `ahead > 0` または `behind == 0`: `git push --force-with-lease`。
6. 後始末: worktree を必ず remove する (失敗経路でも)。

### 排他制御

- 既存 fish 版の mkdir-based mutex は **廃止** し、`fs2` などによる **advisory flock** に置き換える。
- 同じ `$HOME` を使う fish 版と Rust 版は同時並走できない (二重 push を防ぐ)。

## 受け入れ条件

- [ ] 既存 `~/.local/state/resolve-conflicts/queue.json` を読み書きできる (schema 互換)。
- [ ] explicit PR 引数は CONFLICTING フィルタをバイパスする (強制 retry)。
- [ ] no-conflict 経路 (merge が綺麗に通る) でも push は `--force-with-lease`。
- [ ] conflict 経路で agent 異常終了 → `merge --abort` + failed 記録。
- [ ] unmerged が残ったら `merge --abort` + failed 記録。
- [ ] worker 失敗時に worktree が必ず remove される。
- [ ] SIGINT で main loop は break するが in-flight worker は完走待ち。
- [ ] `status` / `reset-failed` が現行 tsv と互換、`--json` も用意。
- [ ] 既存 fish 版と Rust 版を同時に立ち上げると後者が「another instance is running」で exit 1 (lock 衝突)。
- [ ] ログは tail -F できる (append-only / line-buffered)。

## 期待する成果物

- `crates/cmd/rai-cmd-conflicts` crate。
- `rai conflicts {run,status,reset-failed}` を本体に配線。
- README に fish からの移行手順 (`alias resolve-conflicts 'rai conflicts run'`) を記載。

## 非対象

- conflict を agent 抜きで自動解決する戦略 (本 issue は agent 委譲前提)。
- queue schema の変更/migration (今は互換維持)。
