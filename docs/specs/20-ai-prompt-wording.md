# 20 — AI prompt wording

AI engine / agent CLI に渡す文言全体の表現ルール。

## 目的

AI に渡す指示は、対象タスクを最後まで完了する責務を明確に伝える。
別 agent や後続処理に依存してよいように見える表現は含めない。

## 機能要件

- `rai` が生成して AI engine / agent CLI に渡す既定 prompt は、実行中の AI が
  タスクを完了する前提で書く。
- 自動公開や後続処理が内部的に存在する場合でも、それを AI prompt 内で説明しない。
- ユーザーが `--prompt-template` などで明示的に渡した任意文面は対象外とする。

## 受け入れ条件

- [ ] `rai` が生成する既定 prompt に、別 agent や後続処理への依存を示す説明が含まれない。
- [ ] Issue 開発、Issue/PR 再開、PR 修正、conflict 解消、Issue 棚卸しの各 AI prompt が
      タスク完了責務を実行中の AI に向けて直接伝える。

## 非対象

- ログ、CLI help、仕様書、README など、人間向けの説明文。
- ユーザー指定の prompt template。
