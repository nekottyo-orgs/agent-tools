# Timeline の機能マップ

[検証手順](../SKILL.md) の起動・読み取り点検を使う。本物の Herdr と Codex に専用の一時ディレクトリとソケットを割り当て、模擬モデルは localhost で起動する。

| 機能 | 初回に通した入口 |
| --- | --- |
| [Codex の指示表示](codex-history.md) | `herdr agent prompt` → `herdr-timeline --timeline PANE_ID` |
| [pane 一覧](pane-list.md) | `herdr-timeline --json` |
| [接続と履歴の診断](diagnostics.md) | `herdr-timeline --json` と `--timeline PANE_ID` |

公開版と候補版で上の 3 経路を通した。各操作と結果は証跡ディレクトリの `commands.jsonl` に対応する。TUI での選択・移動や Claude Code の履歴は初回の検証範囲外。
