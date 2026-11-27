#!/usr/bin/env bash
# ci-gate の path 選択と gate 集約の回帰テスト。外部 command は bash だけで、どの job からでも走らせられる。
# 実行: repo root から `bash .github/scripts/ci-gate/tests/ci-gate.test.sh`
#
# set -e を外している理由 (why): 1 ケースの失敗で途中終了すると残りのケースが
# 検証されないまま集計されてしまう。全ケースを最後まで走らせて pass/fail を数え上げるため、あえて -e を付けない。
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CI_GATE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DETECT="$CI_GATE_DIR/detect-tools.sh"
CHECK="$CI_GATE_DIR/check-results.sh"
pass=0
fail=0

assert_eq() { # $1=期待値 $2=実際値 $3=説明
  if [ "$1" = "$2" ]; then
    pass=$((pass + 1))
    printf 'OK   %s\n' "$3"
  else
    fail=$((fail + 1))
    printf 'NG   期待 [%s] / 実際 [%s] : %s\n' "$1" "$2" "$3"
  fi
}
assert_contains() { # $1=対象文字列 $2=含まれるべき文字列 $3=説明
  case "$1" in
  *"$2"*)
    pass=$((pass + 1))
    printf 'OK   %s\n' "$3"
    ;;
  *)
    fail=$((fail + 1))
    printf 'NG   "%s" が含まれない : %s (出力=%s)\n' "$2" "$3" "$1"
    ;;
  esac
}

# paths: `git diff -z` と同じ NUL 区切りの入力を渡し、tool ごとの真偽値を比べる
detect_out() {
  printf '%s\0' "$@" | bash "$DETECT"
}
expected_tools() { # $1=bash_gates $2=herdr_timeline $3=inbox_core $4=secret_scan
  printf 'bash_gates=%s\nherdr_timeline=%s\ninbox_core=%s\nsecret_scan=%s\n' "$1" "$2" "$3" "$4"
}
expect_tools() { # $1=説明 $2..$5=期待値 $6...=変更 path
  local desc=$1 bash_gates=$2 herdr_timeline=$3 inbox_core=$4 secret_scan=$5
  shift 5
  assert_eq "$(expected_tools "$bash_gates" "$herdr_timeline" "$inbox_core" "$secret_scan")" \
    "$(detect_out "$@")" "$desc"
}

# 1. tool 配下の変更は、その tool だけ true
expect_tools "paths. bash-gates/ の変更" true false false false 'bash-gates/src/main.rs'
expect_tools "paths. herdr-timeline/ の変更" false true false false 'herdr-timeline/src/main.rs'
expect_tools "paths. inbox-core/ の変更" false false true false 'inbox-core/src/lib.rs'
expect_tools "paths. secret-scan/ の変更" false false false true 'secret-scan/src/main.rs'

# 2. workspace 全体に効く変更は全 tool が true
for file in Cargo.toml Cargo.lock .github/workflows/actions.lock; do
  expect_tools "paths. $file は全 tool" true true true true "$file"
done

# 3. component workflow の変更は、その workflow の tool だけ true
expect_tools "paths. bash-gates.yml の変更" true false false false '.github/workflows/bash-gates.yml'
expect_tools "paths. herdr-timeline.yml の変更" false true false false '.github/workflows/herdr-timeline.yml'
expect_tools "paths. inbox-core.yml の変更" false false true false '.github/workflows/inbox-core.yml'
expect_tools "paths. secret-scan.yml の変更" false false false true '.github/workflows/secret-scan.yml'

# 4. 対象外だけの変更は全 tool が false。ci-gate 自身の変更も tool の suite は呼ばない
expect_tools "paths. 対象外だけの変更" false false false false \
  README.md docs/adr/0001-ci-gate.md .github/workflows/ci-gate.yml \
  .github/scripts/ci-gate/detect-tools.sh
expect_tools "paths. 前方一致の別 directory" false false false false 'bash-gates-old/src/main.rs'

# 5. 非 ASCII・空白・改行を含む path も、そのまま tool の変更として拾う
expect_tools "paths. 非 ASCII の path" false false true false 'inbox-core/日本語/テスト.rs'
expect_tools "paths. 空白を含む path" true false false false 'bash-gates/日本語 ファイル.rs'
expect_tools "paths. 改行を含む path" false false false true $'secret-scan/a\nb.rs'

# 6. 複数 tool の変更は、それぞれ true
expect_tools "paths. 複数 tool の変更" true false true false 'bash-gates/a.rs' 'inbox-core/b.rs'
expect_tools "paths. 変更なし" false false false false

# gate: `job名=result` を渡し、終了コードを比べる
expect_gate() { # $1=期待する終了コード $2=説明 $3...=job名=result
  local want=$1 desc=$2
  shift 2
  bash "$CHECK" "$@" >/dev/null 2>&1
  assert_eq "$want" "$?" "$desc"
}
all_success=(paths=success bash-gates=success herdr-timeline=success inbox-core=success secret-scan=success)

# 7. success と skipped だけなら成功
expect_gate 0 "gate. 全部 success" "${all_success[@]}"
expect_gate 0 "gate. 全部 skipped" \
  paths=skipped bash-gates=skipped herdr-timeline=skipped inbox-core=skipped secret-scan=skipped
expect_gate 0 "gate. success と skipped の混在" \
  paths=success bash-gates=skipped herdr-timeline=success inbox-core=skipped secret-scan=success

# 8. failure / cancelled は 1 つでもあれば失敗し、どの job でも同じ
for job in paths bash-gates herdr-timeline inbox-core secret-scan; do
  for result in failure cancelled; do
    args=("${all_success[@]}")
    for i in "${!args[@]}"; do
      case "${args[$i]}" in
        "$job"=*) args[$i]="$job=$result" ;;
      esac
    done
    expect_gate 1 "gate. $job が $result なら失敗" "${args[@]}"
  done
done

# 9. skipped 以外の未知の result と、結果の渡し忘れ・形式不正も失敗 (fail-open にしない)
expect_gate 1 "gate. 未知の result は失敗" \
  paths=neutral bash-gates=success herdr-timeline=success inbox-core=success secret-scan=success
expect_gate 1 "gate. 結果が無ければ失敗"
expect_gate 1 "gate. 形式が不正なら失敗" paths
expect_gate 1 "gate. 名前が空なら失敗" =success
expect_gate 1 "gate. 結果が空なら失敗" paths=

# 10. 必須 job が欠けると、残りがすべて success でも失敗する
for job in paths bash-gates herdr-timeline inbox-core secret-scan; do
  args=()
  for pair in "${all_success[@]}"; do
    case "$pair" in
      "$job"=*) ;;
      *) args+=("$pair") ;;
    esac
  done
  expect_gate 1 "gate. $job が欠けると失敗" "${args[@]}"
done

# 11. 同じ job が重複していると失敗する (必須 job はすべて揃っていても)
for job in paths bash-gates herdr-timeline inbox-core secret-scan; do
  expect_gate 1 "gate. $job が重複すると失敗" "${all_success[@]}" "$job=success"
done

# 12. 未知の job 名は失敗する。既知 job の typo で置き換わっても失敗する
expect_gate 1 "gate. 未知の job が増えると失敗" "${all_success[@]}" unknown=success
expect_gate 1 "gate. 既知 job 名の typo は失敗" \
  paths=success bash_gate=success herdr-timeline=success inbox-core=success secret-scan=success

# 13. 4 種類の result を 5 job に並べた全組み合わせ (4^5) が真理表と一致する
#     success / skipped だけなら 0、それ以外が 1 つでもあれば 1
values=(success skipped failure cancelled)
mismatches=0
for p in "${values[@]}"; do
  for b in "${values[@]}"; do
    for h in "${values[@]}"; do
      for i in "${values[@]}"; do
        for s in "${values[@]}"; do
          bash "$CHECK" "paths=$p" "bash-gates=$b" "herdr-timeline=$h" "inbox-core=$i" "secret-scan=$s" \
            >/dev/null 2>&1
          rc=$?
          want=0
          for r in "$p" "$b" "$h" "$i" "$s"; do
            case "$r" in
              success|skipped) ;;
              *) want=1 ;;
            esac
          done
          if [ "$rc" -ne "$want" ]; then
            mismatches=$((mismatches + 1))
            printf 'NG   組み合わせ [%s %s %s %s %s] 期待 [%s] / 実際 [%s]\n' \
              "$p" "$b" "$h" "$i" "$s" "$want" "$rc" >&2
          fi
        done
      done
    done
  done
done
assert_eq 0 "$mismatches" "gate. 4^5 の全組み合わせが真理表と一致"

# 14. 失敗した job の名前と result が理由文に出る
out="$(bash "$CHECK" paths=success bash-gates=failure herdr-timeline=skipped \
  inbox-core=cancelled secret-scan=success 2>&1)"
assert_contains "$out" "bash-gates は failure" "gate. failure の job 名が理由文に出る"
assert_contains "$out" "inbox-core は cancelled" "gate. cancelled の job 名が理由文に出る"

# 15. 6 position の例 (モデルの SixPositionsRejected / showSixPositions と対応)。
#     Alloy の elems は位置の重複を畳むため、位置ごとに数えることを実装でも固定する。
#     必須 5 job に、重複した paths か余分な未知の job を足した 6 引数は失敗する
expect_gate 1 "gate. 6 position (paths の重複) は失敗" "${all_success[@]}" paths=success
expect_gate 1 "gate. 6 position (未知の job が余分) は失敗" "${all_success[@]}" unknown=success

printf '結果: pass=%d fail=%d\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
