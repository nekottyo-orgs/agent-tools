# Changelog

## [v0.1.1](https://github.com/nekottyo/agent-tools/compare/herdr-timeline/v0.1.0...herdr-timeline/v0.1.1) - 2026-11-18

### Changes
- ci: PR に tool の label を付け、release notes を label で絞る by @nekottyo in https://github.com/nekottyo/agent-tools/pull/35

## [v0.1.0](https://github.com/nekottyo/agent-tools/commits/herdr-timeline/v0.1.0) - 2026-11-18

### Changes
- feat(herdr-timeline): pane の一覧と Timeline を出す端末 UI と、判定の Lean model を足す by @nekottyo in https://github.com/nekottyo/agent-tools/pull/18
- feat(herdr-timeline): 状態の記号と色を herdr 本体の symbols に揃える by @nekottyo in https://github.com/nekottyo/agent-tools/pull/19
- feat(herdr-timeline): 常駐の pane を一覧から外し、--show-resident で出せるようにする by @nekottyo in https://github.com/nekottyo/agent-tools/pull/20
- feat(herdr-timeline): 監視 skill が起こす PR review の pane も一覧から外す by @nekottyo in https://github.com/nekottyo/agent-tools/pull/21
- fix(herdr-timeline): inbox の帰属 cache で読み取り失敗を残さず、socket は生の値で持つ by @nekottyo in https://github.com/nekottyo/agent-tools/pull/22
- feat(herdr-timeline): 選択行の Timeline を常に出し、呼び出し元の pane を選び、Esc で閉じる by @nekottyo in https://github.com/nekottyo/agent-tools/pull/23
- feat(herdr-timeline): Timeline 列の本文を固定 40 桁で切らず列の幅まで出す by @nekottyo in https://github.com/nekottyo/agent-tools/pull/24
- feat(herdr-timeline): --json で全 pane の状態を機械が読める形で出す by @nekottyo in https://github.com/nekottyo/agent-tools/pull/25
- test(herdr-timeline): 常駐 label の期待値を実装と分け、隠した窓口 pane の名前が残る試験を足す by @nekottyo in https://github.com/nekottyo/agent-tools/pull/26
- fix(herdr-timeline): --json の working_secs を 0 未満にしない by @nekottyo in https://github.com/nekottyo/agent-tools/pull/27
- feat(herdr-timeline): --json に常駐 watcher の状態 (watchers) を足す by @nekottyo in https://github.com/nekottyo/agent-tools/pull/28
- feat(herdr-timeline): --json の pane に画面の異常 (anomaly) を足す by @nekottyo in https://github.com/nekottyo/agent-tools/pull/29
- fix(herdr-timeline): socket が 1 つだけ見つかり環境の socket が無いとき、見つかった server の pane を落とさない by @nekottyo in https://github.com/nekottyo/agent-tools/pull/30
- feat(herdr-timeline): binary release と tagpr の対象に加える by @nekottyo in https://github.com/nekottyo/agent-tools/pull/32
- feat(herdr-timeline): 横並びのとき Timeline を描画幅の 35% にし、一覧は残りを使う by @nekottyo in https://github.com/nekottyo/agent-tools/pull/31
- ci(herdr-timeline): release notes から他 tool の release PR と tool 外の変更を除く by @nekottyo in https://github.com/nekottyo/agent-tools/pull/34
