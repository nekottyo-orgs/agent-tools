# agent-tools

コーディングエージェント (Claude Code など) の運用で使う小さな tool を、tool ごとのディレクトリにまとめる repo。

- tool ごとにディレクトリを分け、build と test の workflow は `.github/workflows/<tool>.yml` に置く。CI の入口は `.github/workflows/ci-gate.yml` に集約し、PR は変更 path に応じて対象 tool の suite だけ、main への push は全 tool の suite を実行する
- Release の workflow は `.github/workflows/release.yml` の 1 本で、tag の prefix (`<tool>/v*`) からどの tool を build するかを決める。tool を足すときは、この workflow の tag の glob と plan job の tools を足す
- release は tagpr で tool ごとに行い、tag は `<tool>/v<version>` 形式 (例: `bash-gates/v0.1.0`)
- herdr-timeline も release の対象で、`herdr-timeline/v<version>` の tag で binary が載る。`cargo install --path herdr-timeline` でも入れられる
- tool は 1 つの Cargo workspace (root の `Cargo.toml`) の member。build の出力は root の `target/` に出る

## tool 一覧

| tool | 概要 |
| --- | --- |
| [bash-gates](bash-gates/) | Bash の PreToolUse hook を束ねる Rust 製の dispatcher。command にきっかけ語がある gate だけを並列に起動し、結果を 1 つにまとめて返す ([設計](bash-gates/docs/adr/0001-dispatcher.md)) |
| [herdr-timeline](herdr-timeline/) | herdr の pane を要対応 / 作業中 / idle・unknown の 3 段で一覧し、pane ごとの Timeline (指示・メッセージ・PR・inbox の報告) と brief の step の進捗を出す Rust 製の端末 UI。読むだけで書き込まない (例外は Enter の `herdr tab focus`) |
| [inbox-core](inbox-core/) | inbox の msg 1 通の操作 (ack / reply / hold など) と観測から、box の移動と副作用の列を判定する Rust 製の CLI。判定は純粋関数で、Lean の model を精密化することを Aeneas で証明している ([証明](inbox-core/lean/README.md)) |
| [secret-scan](secret-scan/) | `git diff` の追加行から、secret と実環境の識別子を検出して JSON で返す Rust 製の単体 binary ([設計](secret-scan/docs/adr/0001-secret-scan-subcommand.md)、[別 binary に分けた理由](secret-scan/docs/adr/0002-separate-binary.md)) |

設計判断の記録 (ADR) は、tool に閉じたものを `<tool>/docs/adr/`、repo 全体のものを `docs/adr/` に置く。書式は [docs/adr/template.md](docs/adr/template.md)。
