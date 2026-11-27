#!/usr/bin/env bash
# PR の変更 path を tool ごとの真偽値に変換する。
# 入力: `git diff --name-only -z` の出力 (NUL 区切り)。path をそのまま扱うため、非 ASCII や改行を含んでも壊れない。
# 出力: GITHUB_OUTPUT に追記できる KEY=value の行。
set -euo pipefail

bash_gates=false
herdr_timeline=false
inbox_core=false
secret_scan=false

# 最後の record が NUL で終わっていなくても取りこぼさない (git -z は必ず NUL で終わるが、入力の形に依存しない)
while IFS= read -r -d '' file || [ -n "$file" ]; do
  [ -n "$file" ] || continue
  case "$file" in
    bash-gates/*|Cargo.toml|Cargo.lock|.github/workflows/bash-gates.yml|.github/workflows/actions.lock)
      bash_gates=true
      ;;
  esac
  case "$file" in
    herdr-timeline/*|Cargo.toml|Cargo.lock|.github/workflows/herdr-timeline.yml|.github/workflows/actions.lock)
      herdr_timeline=true
      ;;
  esac
  case "$file" in
    inbox-core/*|Cargo.toml|Cargo.lock|.github/workflows/inbox-core.yml|.github/workflows/actions.lock)
      inbox_core=true
      ;;
  esac
  case "$file" in
    secret-scan/*|Cargo.toml|Cargo.lock|.github/workflows/secret-scan.yml|.github/workflows/actions.lock)
      secret_scan=true
      ;;
  esac
done

printf 'bash_gates=%s\n' "$bash_gates"
printf 'herdr_timeline=%s\n' "$herdr_timeline"
printf 'inbox_core=%s\n' "$inbox_core"
printf 'secret_scan=%s\n' "$secret_scan"
