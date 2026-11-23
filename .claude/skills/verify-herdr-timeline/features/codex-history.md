# Codex に送った指示を見る

Herdr の Codex pane に送った指示を、指定した pane の Timeline から読める。

## Sub-features

- `codex-history` — 送信した一意の本文の表示

## How to get to it (user POV)

- Herdr の Codex pane から対象の pane ID を取得し、`herdr-timeline --timeline PANE_ID` を実行

## Driving it with Herdr CLI

Preconditions:

- [検証手順](../SKILL.md) の隔離 Herdr、独立 Codex、localhost の模擬モデルと arm64 バイナリ

- [検証手順](../SKILL.md#操作) のスクリプトを実行。`herdr agent prompt PANE_ID TEXT` が返した pane を `herdr pane get PANE_ID` で確認し、`agent_session.source` の `herdr:codex` と ID を取得
- 対応する rollout が一意で、ファイル名と先頭の `session_meta.payload.id` が一致した上で、完了済み UserMessage と `--timeline PANE_ID --width 120` の指示行に同一の検証本文があることを確認

## Gotchas

- 同じ番号の pane が別ソケットに存在する可能性があるため、隔離したソケットを固定
- 共有 backend やフックの信頼待ちで ID が得られない場合は未確認
