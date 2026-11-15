# agent-tools

コーディングエージェント (Claude Code など) の運用で使う小さな tool を、tool ごとのディレクトリにまとめる repo。

- tool ごとにディレクトリを分け、build と test の workflow は `.github/workflows/<tool>.yml` に置いて path filter で絞る
- release は tagpr で tool ごとに行い、tag は `<tool>/v<version>` 形式 (例: `bash-gates/v0.1.0`)
- tool は 1 つの Cargo workspace (root の `Cargo.toml`) の member。build の出力は root の `target/` に出る

## tool 一覧

| tool | 概要 |
| --- | --- |
| [bash-gates](bash-gates/) | Bash の PreToolUse hook を束ねる Rust 製の dispatcher。command にきっかけ語がある gate だけを並列に起動し、結果を 1 つにまとめて返す |
| [secret-scan](secret-scan/) | `git diff` の追加行から、secret と実環境の識別子を検出して JSON で返す Rust 製の単体 binary ([設計](bash-gates/docs/adr/0001-secret-scan-subcommand.md)) |
