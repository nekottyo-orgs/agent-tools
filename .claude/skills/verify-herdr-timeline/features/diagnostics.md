# 接続と履歴の診断を確認する

ソケットに接続できないときは一覧に警告が出る。pane に対応する履歴を開けないときは Timeline にその理由が出る。

## Sub-features

- `socket-unavailable` — 一覧取得に失敗した場合の警告
- `history-unavailable` — 対象 pane に対応する履歴がない場合の診断

## How to get to it (user POV)

- 一覧が空なら `herdr-timeline --json` の `warnings` を確認
- 指示が見つからなければ `herdr pane get PANE_ID` と `herdr-timeline --timeline PANE_ID` を実行

## Driving it with Herdr CLI

Preconditions:

- [検証手順](../SKILL.md) のバイナリと、他のソケットを含まない一時ディレクトリ

- [検証手順](../SKILL.md#操作) のスクリプトを実行。Herdr 起動前の `--json` で `panes` が空かつ `warnings` に `herdr pane list` の失敗が出ることを確認
- 新規 Codex pane を起動し、最初の送信前に `--timeline PANE_ID` の「transcript が見つからない」を確認。起動後の `herdr pane get PANE_ID` で native ID の有無を判定

## Gotchas

- `--json` や `--timeline` が終了コード 0 を返しても警告や履歴なしはあり得るため、出力を照合
- native ID がある場合の履歴なしでは、対応する rollout の欠落・重複・読み取りエラーも確認
