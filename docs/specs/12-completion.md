# 12 — `rai completion`

## 目的

`rai` のサブコマンドツリーに対するシェル補完スクリプトを stdout に出力する。ユーザはその出力をシェル設定に組み込むことで、`rai <TAB>` でサブコマンド・オプションが補完できるようになる。サブコマンドが今後増えても、補完定義を手書きで保守しなくて済む状態を維持する。

## 想定ユーザー

- `rai` を日常的に叩く開発者で、fish / zsh / bash いずれかを使っている人。
- `brew install rai` で入れたあとに、なるべく素直な手順で補完を有効化したい人。

## 機能要件 (何をするか)

- `rai completion <shell>` で指定シェル向けの補完スクリプトを stdout に出す。
- 対応シェル: `bash` / `zsh` / `fish` / `powershell` / `elvish`。
- 補完定義は、`rai` バイナリ本体の clap コマンド定義から自動生成する (サブコマンドや引数の追加に追従する)。
- 出力先は stdout のみ。ファイル書き込みやインストール作業は行わない (ユーザがリダイレクトで好きな場所に置く前提)。
- 既存の `--verbose` などのグローバルオプションも補完に含める。

## CLI 仕様

```
rai completion <shell>

<shell>:
  bash | zsh | fish | powershell | elvish
```

- 終了コード:
  - 正常出力: 0
  - 未知のシェル指定 / 引数不足: clap 既定のエラー出力 + 非 0 (clap が制御する)。

## 受け入れ条件

- [ ] `rai completion fish | source` を fish で実行すると、`rai <TAB>` でサブコマンド一覧 (`hello`, `pair`, `date`, `gh`, `claude`, `dev`, `git`, `pr`, `issue`, `gwq`, `conflicts`, `completion`) が補完される。
- [ ] `rai completion zsh` / `rai completion bash` の出力が、それぞれのシェルで `compinit` / `complete -F` 経由で読み込めて、`rai <TAB>` が動く。
- [ ] サブコマンドを 1 つ追加した後、再ビルドして `rai completion <shell>` を再生成すれば、新サブコマンドが補完候補に出る (= 手書きの補完テーブルが残っていない)。
- [ ] `rai completion` 単体実行 (シェル指定なし) は clap がエラーメッセージを出して非 0 終了する。

## 期待する成果物

- `crates/cmd/rai-cmd-completion` crate (`rai completion` 本体)。
- `clap_complete` を `[workspace.dependencies]` に追加。
- `rai completion` を `rai` 本体に配線。
- README に各シェル向けのインストール手順 (fish / zsh / bash) を追記。

## 非対象

- 補完スクリプトをユーザの設定ディレクトリに直接書き込むインストーラ的挙動。
- `rai completion install` のような糖衣サブコマンド。
- 動的補完 (例: `rai dev pick` のリポジトリ候補をリアルタイム生成する等)。今回はあくまで静的な、サブコマンド/オプション ツリーの補完にとどめる。
