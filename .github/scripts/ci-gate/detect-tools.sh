#!/usr/bin/env bash
# 変更 path を tool ごとの真偽値に変換する。
# 既定 (引数なし): `git diff --name-only -z` の出力 (NUL 区切り) を読む。PR の scoped 実行で使う。
# --all: 入力の path によらず全 tool を true にする。main への push の full 実行で使う。
# 入力の path はそのまま扱うため、非 ASCII や改行を含んでも壊れない。
# 出力: GITHUB_OUTPUT に追記できる KEY=value の行。
set -euo pipefail

if [ "$#" -gt 1 ]; then
  printf 'usage: %s [--all]\n' "$0" >&2
  exit 2
fi

bash_gates=false
herdr_timeline=false
inbox_core=false
secret_scan=false

case "${1-}" in
  '')
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
    ;;
  --all)
    # main への push は、その時点の main (tip) に対して全 suite を実行するため、path では絞らない
    bash_gates=true
    herdr_timeline=true
    inbox_core=true
    secret_scan=true
    ;;
  *)
    printf 'usage: %s [--all]\n' "$0" >&2
    exit 2
    ;;
esac

printf 'bash_gates=%s\n' "$bash_gates"
printf 'herdr_timeline=%s\n' "$herdr_timeline"
printf 'inbox_core=%s\n' "$inbox_core"
printf 'secret_scan=%s\n' "$secret_scan"
