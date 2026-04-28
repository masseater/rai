# 12 — `rai completion`

## 目的

`rai` のサブコマンドツリーに対するシェル補完スクリプトを stdout に出力する。ユーザはその出力をシェル設定に組み込むことで、`rai <TAB>` でサブコマンド・オプションが補完できるようになる。サブコマンドが今後増えても、補完定義を手書きで保守しなくて済む状態を維持する。

永続設定では、生成済みスクリプトを固定ファイルとして置くだけでなく、シェル起動時に現在の `rai` バイナリから補完を読み込める導線も提供する。これにより `rai` を更新した後、新しいシェルでは常に更新後バイナリの補完が適用される。

## 想定ユーザー

- `rai` を日常的に叩く開発者で、fish / zsh / bash いずれかを使っている人。
- `brew install rai` で入れたあとに、なるべく素直な手順で補完を有効化したい人。

## 機能要件 (何をするか)

- `rai completion <shell>` で指定シェル向けの補完スクリプトを stdout に出す。
- `rai completion --source <shell>` で、シェル起動時に現在の `rai completion <shell>` を読み込むための設定スニペットを stdout に出す。
- 対応シェル: `bash` / `zsh` / `fish` / `powershell` / `elvish`。
- 補完定義は、`rai` バイナリ本体の clap コマンド定義から自動生成する (サブコマンドや引数の追加に追従する)。
- 出力先は stdout のみ。ファイル書き込みやインストール作業は行わない (ユーザがリダイレクトで好きな場所に置く前提)。
- 既存の `--verbose` などのグローバルオプションも補完に含める。

## CLI 仕様

```
rai completion <shell>
rai completion --source <shell>

<shell>:
  bash | zsh | fish | powershell | elvish
```

- 終了コード:
  - 正常出力: 0
  - 未知のシェル指定 / 引数不足: clap 既定のエラー出力 + 非 0 (clap が制御する)。
- `--source` の出力は、ユーザが各シェルの rc/config に置ける短いスニペットである。
- `--source` は stdout にスニペットを出すだけで、ユーザの設定ファイルを書き換えない。

## 受け入れ条件

- [ ] `rai completion fish | source` を fish で実行すると、`rai <TAB>` でサブコマンド一覧 (`hello`, `pair`, `date`, `gh`, `claude`, `dev`, `git`, `pr`, `issue`, `gwq`, `conflicts`, `completion`) が補完される。
- [ ] `rai completion zsh` / `rai completion bash` の出力が、それぞれのシェルで `compinit` / `complete -F` 経由で読み込めて、`rai <TAB>` が動く。
- [ ] サブコマンドを 1 つ追加した後、再ビルドして `rai completion <shell>` を再生成すれば、新サブコマンドが補完候補に出る (= 手書きの補完テーブルが残っていない)。
- [ ] `rai completion --source fish` の出力を fish 設定に置くと、新しいシェル起動時に現在の `rai` バイナリから補完が読み込まれる。
- [ ] `rai completion --source zsh` / `rai completion --source bash` の出力を各シェル設定に置くと、新しいシェル起動時に現在の `rai` バイナリから補完が読み込まれる。
- [ ] `rai completion` 単体実行 (シェル指定なし) は clap がエラーメッセージを出して非 0 終了する。

## 期待する成果物

- `crates/cmd/rai-cmd-completion` crate (`rai completion` 本体)。
- `clap_complete` を `[workspace.dependencies]` に追加。
- `rai completion` を `rai` 本体に配線。
- README に各シェル向けの永続読み込み手順 (fish / zsh / bash) を追記。

## 非対象

- 補完スクリプトをユーザの設定ディレクトリに直接書き込むインストーラ的挙動。
- `rai completion install` のような糖衣サブコマンド。
- 動的補完 (例: `rai dev pick` のリポジトリ候補をリアルタイム生成する等)。今回はあくまで静的な、サブコマンド/オプション ツリーの補完にとどめる。
