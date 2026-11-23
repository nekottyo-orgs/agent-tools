# Changelog

## [v0.2.5](https://github.com/nekottyo/agent-tools/compare/inbox-core/v0.2.4...inbox-core/v0.2.5) - 2026-11-23

### Changes
- refactor(inbox-core): lane の出力整形を共通化する by @nekottyo in https://github.com/nekottyo/agent-tools/pull/74
- feat(inbox-core): 停止種別ごとの上限 T (F1=3分 / F2=15分) を追加する by @nekottyo in https://github.com/nekottyo/agent-tools/pull/85

## [v0.2.4](https://github.com/nekottyo/agent-tools/compare/inbox-core/v0.2.3...inbox-core/v0.2.4) - 2026-11-22

### Changes
- fix(inbox-core): 非実装タスクの新しい判断待ちを表示する by @nekottyo in https://github.com/nekottyo/agent-tools/pull/63

## [v0.2.3](https://github.com/nekottyo/agent-tools/compare/inbox-core/v0.2.2...inbox-core/v0.2.3) - 2026-11-21

### Changes
- test: 形式検証を先に更新し CI で証明を確認する by @nekottyo in https://github.com/nekottyo/agent-tools/pull/57

## [v0.2.2](https://github.com/nekottyo/agent-tools/compare/inbox-core/v0.2.1...inbox-core/v0.2.2) - 2026-11-20

### Changes
- feat(inbox-core): 依頼の種類と終わりの印で done を決める判定を足す by @nekottyo in https://github.com/nekottyo/agent-tools/pull/52

## [v0.2.1](https://github.com/nekottyo/agent-tools/compare/inbox-core/v0.2.0...inbox-core/v0.2.1) - 2026-11-20

### Changes
- feat(inbox-core): stalled の生成物が Nat の上の仕様に一致することを証明する by @nekottyo in https://github.com/nekottyo/agent-tools/pull/46
- feat(inbox-core): resend_step と next_lane の生成物についての定理を足す by @nekottyo in https://github.com/nekottyo/agent-tools/pull/48
- feat(inbox-core): 上限 T の 1 回目を worker の差し替えに回す遷移を足す by @nekottyo in https://github.com/nekottyo/agent-tools/pull/51

## [v0.2.0](https://github.com/nekottyo/agent-tools/compare/inbox-core/v0.1.0...inbox-core/v0.2.0) - 2026-11-20

### Changes
- fix(inbox-core): handoff model の穴 3 件を直す (crash 後の token、fire の guard、E5 の見せた内容) by @nekottyo in https://github.com/nekottyo/agent-tools/pull/43
- feat(inbox-core): lane・停止・再送の段の判定を純粋関数と CLI に足す by @nekottyo in https://github.com/nekottyo/agent-tools/pull/45

## [v0.1.0](https://github.com/nekottyo/agent-tools/compare/herdr-timeline/v0.1.0...inbox-core/v0.1.0) - 2026-11-20

### Changes
- feat(inbox-core): inbox の判定の crate と Lean の証明を足し、binary release の対象に加える by @nekottyo in https://github.com/nekottyo/agent-tools/pull/40
