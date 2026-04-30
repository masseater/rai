# 15 — `rai issue triage`

## 目的

`rai issue inventory --apply` で triage ラベルとコメントを焼き込んだ後の **人間レビュー段階** を担う。ユーザーは Issue 1 件ずつの本文・コメントを見ながら「close するか/しないか」だけを判断し、それ以外の機械的な操作 (close 実行、ラベル削除) は `rai` がまとめて適用する。

## 機能要件

- `rai issue triage` として提供する。
- 対象リポジトリ:
  - 未指定時は現在の GitHub リポジトリ。
  - `--repo OWNER/REPO` で上書きできる。
- レビュー対象:
  - 既定では label `triage:close-candidate` が付いた open Issue。
  - `--label LABEL` で他のラベルに切り替え可能 (例: `triage:duplicate`, `triage:stale`)。
- レビュー UI:
  - 1 件ずつ進める。各 Issue について:
    - ヘッダ (現在位置 [N/total], `#番号 — タイトル`) を表示。
    - `gh issue view <N> --repo R --comments` を inherited stdio で起動し、本文と全コメント (rai issue inventory が投稿した判定コメントを含む) を gh の整形付き出力で表示する。
    - 続けてプロンプトを出し、ユーザーは以下のキーで判断する:
      - `c`: close する
      - `k`: close しない (label は外す)
      - `s`: 判断保留 (このセッションでは何もしない、label もコメントもそのまま)
      - `q`: 中断 (ここまでに行った判断を **すべて破棄** して終了)
    - 不正入力は再プロンプトする。EOF (Ctrl-D など) は `q` と同じ扱い。
- 適用:
  - すべての Issue を巡回し終えたら、判断順に以下を実行:
    - close 判断: `gh issue close <N> --repo R --reason <REASON> [--comment <BODY>]`
    - keep 判断: `gh issue edit <N> --repo R --remove-label <LABEL>` (`--keep-label-on-keep` 指定時はスキップ)
    - skip: 何もしない
  - 最後に `closed=N kept=N skipped=N` のサマリを stderr に出す。
- オプション:
  - `--reason completed|not-planned`: 既定 `completed`。
  - `--close-comment "..."`: close 時に共通コメントを投稿する。
  - `--keep-label-on-keep`: keep 判断時にラベルを残す (deferred 用途)。

## 受け入れ条件

- [ ] `rai issue triage --repo R` が triage:close-candidate 付き open issue を 1 件ずつ表示する。
- [ ] 表示には body と全コメントが含まれる (`gh issue view --comments` を経由)。
- [ ] プロンプトは `c/k/s/q` を受け付け、不正入力は再プロンプトされる。EOF は `q` と同じ。
- [ ] 全 Issue 巡回後、close/keep/skip がまとめて適用される。
- [ ] `q` で中断した場合、それまでの判断は **適用されない**。
- [ ] `--reason not-planned` で gh の close 理由を切り替えられる。
- [ ] `--keep-label-on-keep` で keep 判断時もラベルが残る。

## 期待する成果物

- `crates/cmd/rai-cmd-issue` に `triage` サブコマンドを追加。
- `rai issue triage` を `rai issue` の dispatcher に配線。
- README に `rai issue inventory --apply` → `rai issue triage` のフローを記載。

## 非対象

- 複数ラベルの同時レビュー (1 回の起動につき 1 ラベル)。
- 既存コメントの編集や削除。
- close 後の追加 post-processing (例: マイルストーン整理)。
