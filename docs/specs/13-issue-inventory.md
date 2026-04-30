# 13 — `rai issue inventory`

## 目的

GitHub Issue の棚卸しを行い、AI による判断結果を **GitHub Issue 自体に「コメント + ラベル」として焼き込む** ことで、ユーザーが後から AI に依存せず機械的に (例: `gh issue list --label triage:close-candidate` で抽出して一括 close) 処理できる状態にする。

Issue 取得と書き込み (コメント投稿・ラベル付与) は `rai` の責務とし、AI engine には GitHub アクセスを行わせない。

## 機能要件

- `rai issue inventory` として提供する。
- 対象リポジトリ:
  - 未指定時は現在の GitHub リポジトリを使う。
  - `--repo OWNER/REPO` で上書きできる。
- Issue 取得:
  - `gh issue list` を使って Issue を取得する。
  - 既定では open Issue を最大 100 件取得する。
  - `--state open|closed|all` で状態を指定できる。
  - `--limit N` で取得件数を指定できる。
  - `--label LABEL`、`--assignee LOGIN`、`--author LOGIN`、`--search QUERY` で絞り込める。
- AI engine 実行:
  - `-e/--engine-cmd CMD` で engine CLI を指定できる。
  - 既定の engine は既存 agent 系コマンドと同じ `ccs_print c1`。
  - 固定プロンプトには取得済み Issue JSON、取得条件、verdict JSON の schema を含める。
  - 固定プロンプトでは、AI engine が Issue 取得や `gh issue list/view` を実行しないこと、書き込み (close, label 変更) も `rai` が行うので AI 側では行わないことを明示する。
  - 既定では prompt を engine CLI の最後の引数として渡す。
  - `--prompt-stdin` 指定時は prompt を標準入力で渡す。
  - engine の標準出力は `rai` が capture し、ユーザーにもリアルタイムで表示する。
- Verdict 抽出:
  - engine 出力から最初の `` ```json `` フェンスブロックを取り出して verdict JSON としてパースする。
  - schema:
    ```json
    {
      "verdicts": [
        {
          "number": 42,
          "category": "close-candidate",
          "summary": "1行の要約",
          "reason": "Markdown 詳細理由",
          "labels": ["triage:close-candidate"]
        }
      ]
    }
    ```
  - `category` は AI への指示として `close-candidate` / `duplicate` / `stale` / `needs-info` / `keep` / `split` を推奨するが、文字列としてはそのまま受け取る。
  - `labels` には必ず `triage:` で始まるラベルを 1 つ以上含めるよう prompt で指示する。
- 適用 (`--apply`):
  - 既定は dry-run。verdicts を整形して標準出力にプレビューするだけ。
  - `--apply` を付けた場合のみ各 Issue に対して以下を実行する:
    1. verdict 中の `labels` のうち、リポジトリに存在しないものを `gh label create` で作成する (色は自動)。
    2. `gh issue edit <number> --add-label <labels...>` でラベルを付与する。
    3. `gh issue comment <number> --body <body>` で AI の判定をコメントする。コメント末尾には `<!-- rai-issue-inventory -->` のマーカー HTML コメントを含める (将来の重複検知用)。
- Verdict 永続化:
  - `--save-verdicts FILE` で engine の生出力をファイル保存できる。
  - `--from-verdicts FILE` で engine を起動せずファイルから読み込んで `--apply` などを実行できる。指定時は engine 関連オプション (`--engine-cmd`, `--prompt-stdin`, `--print-prompt`) と排他。
- `--print-prompt` 指定時は engine を起動せず、生成した prompt を stdout に出力する。

## 受け入れ条件

- [ ] `rai issue inventory --repo OWNER/REPO` が `gh issue list` で Issue を取得できる。
- [ ] `--state`、`--limit`、絞り込みオプションが `gh issue list` に反映される。
- [ ] 生成 prompt に取得済み Issue JSON と verdict JSON schema が含まれる。
- [ ] 生成 prompt に「AI engine が Issue 取得・書き込みを行わない」制約が含まれる。
- [ ] engine 出力から `` ```json `` ブロックを抽出して verdict をパースできる。
- [ ] 既定 (dry-run) では各 verdict のプレビューだけが標準出力に出る。
- [ ] `--apply` を付けると `gh label create` (必要なら) → `gh issue edit --add-label` → `gh issue comment` の順で各 Issue に書き込みが行われる。
- [ ] `--save-verdicts FILE` で engine 生出力が保存される。
- [ ] `--from-verdicts FILE` で保存済み出力から再適用できる。
- [ ] `--print-prompt` で engine を起動せず prompt を確認できる。

## 期待する成果物

- `crates/cmd/rai-cmd-issue` に `inventory` サブコマンドを追加。
- `rai issue inventory` を `rai issue` の dispatcher に配線。
- README に `rai issue inventory` の用途と「label でフィルタして機械的に close する」フローを記載。

## 非対象

- AI engine の認証・設定。
- AI engine が GitHub にアクセスして追加調査するワークフロー。
- 既存コメントの **更新** (常に新規コメントを追加する。古いコメントの整理はユーザー責任)。
- close 操作。`rai issue inventory` 自体はラベル付与とコメント投稿までで止める。close は対になる subcommand `rai issue triage` (spec 15) で人間レビュー付きで行う。
