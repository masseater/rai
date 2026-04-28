# 05 — `rai dev`

Source issue: [#5](https://github.com/masseater/rai/issues/5)

## 目的

`ghq` + `gwq` で管理しているリポジトリ/worktree から fzf で 1 つを選び、選択結果のパスを stdout に出す。bin から親シェルの cwd は変えられないので、選択結果を吐くだけにとどめ、cd / tmux rename はシェル側 wrapper の責務とする。

## 機能要件

- 候補集合: `ghq list --full-path` ∪ `gwq list --full-path` (重複排除、順序安定)。
- `gwq` が無い環境では ghq だけで動く。
- 既定では `--filter <regex>` 相当のフィルタを掛ける (現行 `_dev_repo_id` 相当の用途を想定)。
- `--all` 指定時は全候補を出す。
- 表示は `ghq root`/github.com/ プレフィクスを取り除いた人間向けラベルに。
- fzf を spawn し、選択結果のフルパスを stdout に 1 行で出す。
- 終了コード:
  - 候補なし: 0 (stdout 空)
  - fzf キャンセル: 130
  - 選択成功: 0

## fish wrapper 仕様

bin 単独では cd / tmux rename を起こせないので、fish 側に下記 wrapper を提供する:

```fish
function dev --description "ghq/gwq + fzf"
    set -l target (rai dev $argv)
    test -n "$target"; and cd $target
    set -q TMUX; and tmux rename-session (basename $target | string replace -a . -)
end
```

## 受け入れ条件

- [ ] `rai dev` が選択結果のフルパスを 1 行で stdout に出す。
- [ ] 候補なしで exit 0 (silent)、fzf キャンセルで exit 130。
- [ ] `--all` の有無で挙動が現行 fish 版と一致。
- [ ] fish 側 wrapper を提供し、cd + tmux rename-session が動く。
- [ ] gwq が無い環境でも ghq だけで動く。

## 期待する成果物

- `crates/cmd/rai-cmd-dev` crate (`rai dev`)。
- `rai dev` を `rai` 本体に配線。
- README に fish wrapper のサンプルを記載 (移行手順込み)。

## 非対象

- bin 側で直接 cd / tmux rename することは諦める (シェルプロセスを変更できないため)。
