#!/usr/bin/env bash
# gate job の集約判定。引数は `job名=result` の形で受ける。
# success と skipped だけを成功として扱い、failure / cancelled / 未知の result は失敗にする。
# 引数が無い・形式が不正なときも、設定の取りこぼしを成功にしないため失敗にする。
set -uo pipefail

if [ "$#" -eq 0 ]; then
  echo "job の結果が 1 つも渡されていない" >&2
  exit 1
fi

failed=0
for pair in "$@"; do
  case "$pair" in
    *=*) ;;
    *)
      echo "結果の指定が不正: $pair" >&2
      failed=1
      continue
      ;;
  esac
  name="${pair%%=*}"
  result="${pair#*=}"
  if [ -z "$name" ] || [ -z "$result" ]; then
    echo "結果の指定が不正: $pair" >&2
    failed=1
    continue
  fi
  case "$result" in
    success|skipped) ;;
    *)
      echo "$name は $result で終わった" >&2
      failed=1
      ;;
  esac
done

exit "$failed"
