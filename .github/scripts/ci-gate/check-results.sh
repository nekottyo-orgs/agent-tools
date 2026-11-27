#!/usr/bin/env bash
# gate job の集約判定。引数は `job名=result` の形で受ける。
# success と skipped だけを成功として扱い、failure / cancelled / 未知の result は失敗にする。
# 引数が無い・形式が不正なときも、設定の取りこぼしを成功にしないため失敗にする。
# 必須 job の集合とちょうど一致することを要求する。欠落・重複・未知の job 名はすべて失敗にする
# (呼び出し側で結果が抜けた、名前を typo した、といった fail-open を防ぐ)。
set -uo pipefail

required_jobs=(paths bash-gates herdr-timeline inbox-core secret-scan)

if [ "$#" -eq 0 ]; then
  echo "job の結果が 1 つも渡されていない" >&2
  exit 1
fi

failed=0
seen=" "
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

  known=0
  for job in "${required_jobs[@]}"; do
    if [ "$name" = "$job" ]; then
      known=1
      break
    fi
  done
  if [ "$known" -ne 1 ]; then
    echo "未知の job: $name" >&2
    failed=1
    continue
  fi

  case "$seen" in
    *" $name "*)
      echo "job が重複している: $name" >&2
      failed=1
      continue
      ;;
  esac
  seen="$seen$name "

  case "$result" in
    success|skipped) ;;
    *)
      echo "$name は $result で終わった" >&2
      failed=1
      ;;
  esac
done

for job in "${required_jobs[@]}"; do
  case "$seen" in
    *" $job "*) ;;
    *)
      echo "job の結果が無い: $job" >&2
      failed=1
      ;;
  esac
done

exit "$failed"
