# pane の一覧を読む

JSON 出力で、起動した Herdr の pane と警告を確認できる。

## Sub-features

- `pane-list-empty` — 正常な空の一覧
- `pane-list-present` — 対象 pane がある一覧

## How to get to it (user POV)

- 端末から `herdr-timeline --json` を実行

## Driving it with Herdr CLI

Preconditions:

- [検証手順](../SKILL.md) の新規 Herdr ソケットと arm64 バイナリ

- [検証手順](../SKILL.md#操作) のスクリプトを実行。起動直後の `herdr pane list` と `herdr-timeline --json` で `panes` が空、Timeline の `warnings` も空であることを確認
- workspace を作成し Codex へ送信した後、再度 `--json` で対象の `pane_id` があり、`warnings` が空であることを確認

## Gotchas

- 警告がある空一覧は [接続と履歴の診断](diagnostics.md) の対象
