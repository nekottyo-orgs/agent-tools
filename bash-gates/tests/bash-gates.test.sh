#!/usr/bin/env bash
# bash-gates (Rust の親 hook) の回帰テスト。
# 子は本物を使わず、BASH_GATES_DIR に置いた偽の子と小さな manifest で、きっかけ語の照合と合成規則を検証する。
# バイナリのビルドが要るので、cargo が無ければ理由を出して skip する。lean が無ければ Lean の検査だけ skip する。
# 実行: repo root から `bash bash-gates/tests/bash-gates.test.sh`
#
# set -e を外している理由 (why): 1 ケースの失敗で途中終了すると残りのケースが
# 検証されないまま集計されてしまう。全ケースを最後まで走らせて pass/fail を数え上げるため、あえて -e を付けない。
set -uo pipefail

# 対象はテストファイルからの相対で解決する (worktree 並走時に別コピーを検証しないため)
TOOL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
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
assert_empty() { # $1=実際値(標準出力) $2=説明
  if [ -z "$1" ]; then
    pass=$((pass + 1))
    printf 'OK   %s\n' "$2"
  else
    fail=$((fail + 1))
    printf 'NG   出力が空でない : %s (出力=%s)\n' "$2" "$1"
  fi
}

# Lean: 証明が warning (sorry を含む) なしで通り、モデルから作り直した表が commit 済みの表と同じ。
# cargo test は表と Rust を比べるだけなので、ここで見ないとモデルや証明が壊れても気づけない。lean が無ければ skip
LEAN_DIR="$TOOL_ROOT/lean"
if command -v lean >/dev/null 2>&1; then
  lean_tmp="$(mktemp -d)"
  lean_out="$(lean -R "$LEAN_DIR" -o "$lean_tmp/BashGates.olean" "$LEAN_DIR/BashGates.lean" 2>&1)"
  assert_eq 0 "$?" "lean. 証明が通る"
  # Lean 自身の診断 (file:行:列: warning:) だけを見る。elan の proxy は toolchain の場所によって
  # "warning: could not canonicalize path" を stderr に出すが、証明とは無関係
  assert_eq '' "$(printf '%s\n' "$lean_out" | grep -E ':[0-9]+:[0-9]+: (warning|error)|sorryAx|ofReduceBool')" "lean. warning と sorry と native_decide が無い"
  # Vectors.lean は cwd からの相対パス bash-gates/lean/vectors.txt に書く。実物を上書きしないよう一時領域で作る
  mkdir -p "$lean_tmp/bash-gates/lean"
  (cd "$lean_tmp" && LEAN_PATH="$lean_tmp" lean --run "$LEAN_DIR/Vectors.lean" >/dev/null 2>&1)
  cmp -s "$lean_tmp/bash-gates/lean/vectors.txt" "$LEAN_DIR/vectors.txt"
  assert_eq 0 "$?" "lean. モデルから作り直した表が vectors.txt と一致"
  rm -rf "$lean_tmp"
else
  echo "lean が無いので Lean の証明検査を skip"
fi

CARGO="$(command -v cargo || true)"
if [ -z "$CARGO" ]; then
  echo "cargo が無いので skip"
  printf '結果: pass=%d fail=%d\n' "$pass" "$fail"
  [ "$fail" -eq 0 ]
  exit $?
fi

if ! "$CARGO" build --release --offline --quiet --manifest-path "$TOOL_ROOT/Cargo.toml"; then
  echo "NG   bash-gates のビルドに失敗"
  exit 1
fi
# workspace の root の target/ に出る
BIN="$TOOL_ROOT/../target/release/bash-gates"

# 0. 関数単位で Lean のモデル (lean/BashGates.lean) と一致する。表は lean/vectors.txt、比べるのは main.rs の #[cfg(test)]
"$CARGO" test --release --offline --quiet --manifest-path "$TOOL_ROOT/Cargo.toml" >/dev/null 2>&1
assert_eq 0 "$?" "0. cargo test (Lean のモデルとの突き合わせ) が通る"

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
CHILD_DIR="$TMP_ROOT/children"
CTL="$TMP_ROOT/ctl"
MANIFEST="$TMP_ROOT/manifest.json"
export BASH_GATES_DIR="$CHILD_DIR" BASH_GATES_MANIFEST="$MANIFEST" BASH_GATES_FRESH=1 FAKE_CTL="$CTL"

NAMES="a b c sub gh1 slow"
mkdir -p "$CHILD_DIR"
cat >"$TMP_ROOT/fake.sh" <<'FAKE'
#!/usr/bin/env bash
n="$(basename "$0" .sh)"
cat >/dev/null
touch "$FAKE_CTL/$n.ran"
[ -f "$FAKE_CTL/$n.sleep" ] && sleep "$(cat "$FAKE_CTL/$n.sleep")"
[ -f "$FAKE_CTL/$n.err" ] && cat "$FAKE_CTL/$n.err" >&2
[ -f "$FAKE_CTL/$n.out" ] && cat "$FAKE_CTL/$n.out"
exit "$(cat "$FAKE_CTL/$n.rc" 2>/dev/null || echo 0)"
FAKE
for n in $NAMES; do
  cp "$TMP_ROOT/fake.sh" "$CHILD_DIR/$n.sh"
  chmod +x "$CHILD_DIR/$n.sh"
done
cat >"$MANIFEST" <<'J'
{"dispatched":[
 {"command":"a.sh","via":"bash-gates.sh","timeout":10,"words":["git"]},
 {"command":"b.sh","via":"bash-gates.sh","timeout":10,"words":["git"]},
 {"command":"c.sh","via":"bash-gates.sh","timeout":10,"words":["git"]},
 {"command":"sub.sh","via":"bash-gates.sh","timeout":10,"substrings":["SUB_SKIP"]},
 {"command":"gh1.sh","via":"bash-gates.sh","timeout":10,"words":["gh"]},
 {"command":"slow.sh","via":"bash-gates.sh","timeout":1,"words":["slowcmd"]},
 {"command":"other.sh","via":"another.sh","words":["git"]}]}
J

reset() { rm -rf "$CTL"; mkdir -p "$CTL"; }
decision() { # $1=名前 $2=deny|ask $3=理由
  jq -cn --arg d "$2" --arg r "$3" '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:$d,permissionDecisionReason:$r}}' >"$CTL/$1.out"
}
run() { # $1=Bash の command。終了コードは $RC に入る
  jq -cn --arg c "$1" '{tool_name:"Bash",tool_input:{command:$c}}' | "$BIN" 2>/dev/null
  RC=${PIPESTATUS[1]}
}
run_raw() { printf '%s' "$1" | "$BIN" 2>/dev/null; RC=$?; }
# $(run ...) は subshell なので RC が戻らない。終了コードを見るケースはこちらで stdout を $out に受ける
runo() { run "$1" >"$TMP_ROOT/stdout"; out="$(cat "$TMP_ROOT/stdout")"; }
ran() { # 起動された子の名前を空白区切りで返す
  local n r=""
  for n in $NAMES; do [ -e "$CTL/$n.ran" ] && r="$r $n"; done
  printf '%s' "${r# }"
}
reason() { jq -r '.hookSpecificOutput.permissionDecisionReason' <<<"$1"; }

# 1. きっかけ語なしは子を 1 本も起動せず stdout 空
for c in 'ls -la' 'cat github.com/x' 'ls /home/x/.gh/worktrees/foo'; do
  reset; out="$(run "$c")"
  assert_empty "$out" "1. [$c] は stdout 空"
  assert_eq "" "$(ran)" "1. [$c] は子を起動しない"
done

# 2. words の境界
for c in 'git status' 'cd x && git commit' 'bash -c "git commit"' 'g""it add -A' '\git add -A' '/usr/bin/git log'; do
  reset; run "$c" >/dev/null
  assert_eq "a b c" "$(ran)" "2. [$c] は git の子を起動する"
done
for c in 'git-lfs ls-files' 'legit run'; do
  reset; run "$c" >/dev/null
  assert_eq "" "$(ran)" "2. [$c] は起動しない"
done

# 3. substrings
reset; run 'SUB_SKIP=1 make' >/dev/null
assert_eq "sub" "$(ran)" "3. substrings で該当の子だけ起動"
reset; run 'gh pr view 1' >/dev/null
assert_eq "gh1" "$(ran)" "3. gh は gh の子だけ起動 (via が別の子は対象外)"

# 4. 壊れた入力は全員起動 (via が違う子は除く)
reset; run_raw '{broken' >/dev/null
assert_eq "a b c sub gh1 slow" "$(ran)" "4. 入力 JSON が壊れていれば全員起動"
reset; run_raw '{"tool_input":{"command":123}}' >/dev/null
assert_eq "a b c sub gh1 slow" "$(ran)" "4. command が文字列でなければ全員起動"

# 5. manifest が読めなければ deny、exit 0
reset
BASH_GATES_MANIFEST="$TMP_ROOT/none.json" runo 'git status'
assert_eq "deny" "$(jq -r '.hookSpecificOutput.permissionDecision' <<<"$out")" "5. manifest が読めなければ deny"
assert_eq 0 "$RC" "5. manifest が読めなくても exit 0"
printf '{broken' >"$TMP_ROOT/bad.json"
out="$(BASH_GATES_MANIFEST="$TMP_ROOT/bad.json" run 'git status')"
assert_eq "deny" "$(jq -r '.hookSpecificOutput.permissionDecision' <<<"$out")" "5. manifest が壊れていても deny"
echo '{"dispatched":[{"command":"other.sh","via":"another.sh","words":["git"]}]}' >"$TMP_ROOT/nochild.json"
out="$(BASH_GATES_MANIFEST="$TMP_ROOT/nochild.json" run 'ls')"
assert_eq "deny" "$(jq -r '.hookSpecificOutput.permissionDecision' <<<"$out")" "5. 束ねる子が 0 本なら deny"
reset
TMPDIR="$TMP_ROOT/none-dir" runo 'git status'
assert_eq "deny" "$(jq -r '.hookSpecificOutput.permissionDecision' <<<"$out")" "5. 一時領域を作れなければ deny"
assert_eq 0 "$RC" "5. 一時領域を作れなくても exit 0"
assert_eq "" "$(ran)" "5. 一時領域を作れなければ子を起動しない"
reset; chmod -x "$CHILD_DIR/b.sh"; decision a deny "理由A"
runo 'git status'
chmod +x "$CHILD_DIR/b.sh"
assert_eq "deny" "$(jq -r '.hookSpecificOutput.permissionDecision' <<<"$out")" "5. 起動できない子があれば deny"
assert_contains "$(reason "$out")" "b.sh を起動できない" "5. 起動できない子の名前が理由文に出る"
assert_contains "$(reason "$out")" "理由A" "5. 起動できた子の deny 理由も残る"
assert_eq 0 "$RC" "5. 子を起動できなくても exit 0"

# 6. 合成
reset; decision a deny "理由A"; decision b deny "理由B"
runo 'git status'
assert_eq $'[a]\n理由A\n\n[b]\n理由B' "$(reason "$out")" "6. deny 2 本は見出し付きで連結"
assert_eq 0 "$RC" "6. 終了コードは 0"

reset; decision a deny "同文"; decision b deny "同文"
out="$(run 'git status')"
assert_eq "同文" "$(reason "$out")" "6. 同じ理由文は 1 つ"

reset; decision a ask "確認A"; decision b deny "拒否B"
out="$(run 'git status')"
assert_eq "deny" "$(jq -r '.hookSpecificOutput.permissionDecision' <<<"$out")" "6. deny と ask 混在は deny"
assert_eq "拒否B" "$(reason "$out")" "6. deny 側の理由文だけ残る"

reset; decision a ask "確認A"
out="$(run 'git status')"
assert_eq "ask" "$(jq -r '.hookSpecificOutput.permissionDecision' <<<"$out")" "6. ask のみは ask"

reset
echo '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"文脈A"}}' >"$CTL/a.out"
echo '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"文脈B"},"systemMessage":"通知B"}' >"$CTL/b.out"
out="$(run 'git status')"
assert_eq $'文脈A\n\n文脈B' "$(jq -r '.hookSpecificOutput.additionalContext' <<<"$out")" "6. additionalContext を連結"
assert_eq "null" "$(jq -r '.hookSpecificOutput.permissionDecision // "null"' <<<"$out")" "6. context だけなら判定なし"
assert_eq "通知B" "$(jq -r '.systemMessage' <<<"$out")" "6. systemMessage をトップレベルに出す"

reset; echo "stderr の理由" >"$CTL/a.err"; echo 2 >"$CTL/a.rc"
out="$(run 'git status')"
assert_eq "deny" "$(jq -r '.hookSpecificOutput.permissionDecision' <<<"$out")" "6. exit 2 は deny"
assert_contains "$(reason "$out")" "stderr の理由" "6. exit 2 は stderr が理由文"

reset; echo 1 >"$CTL/a.rc"; echo "壊れた" >"$CTL/b.out"
decision c deny "理由C"
out="$(run 'git status')"
assert_eq "理由C" "$(reason "$out")" "6. exit 1 と壊れた JSON の子は素通りで他の子は生きる"
reset; echo 1 >"$CTL/a.rc"; echo "壊れた" >"$CTL/b.out"
out="$(run 'git status')"
assert_empty "$out" "6. 全員素通りなら stdout 空"

# 7. timeout: 1 秒の子が 5 秒 sleep しても素通りで、全体は 5 秒未満
reset; echo 5 >"$CTL/slow.sleep"; decision slow deny "遅い"
SECONDS=0
runo 'slowcmd'
assert_empty "$out" "7. timeout した子は素通り"
assert_eq 1 "$([ "$SECONDS" -lt 5 ] && echo 1 || echo 0)" "7. 全体が 5 秒未満で返る (${SECONDS} 秒)"
assert_eq 0 "$RC" "7. 終了コードは 0"

# 9. バイナリが source より新しくなければ、stdin を読まずに退避側へ exec する。FRESH があれば比べない
ST="$TMP_ROOT/stale"
mkdir -p "$ST/bash-gates/src"
touch -t 203001010000 "$ST/bash-gates/src/main.rs"
printf '#!/bin/sh\nprintf "fallback:"; cat\n' >"$ST/bash-gates.sh"
chmod +x "$ST/bash-gates.sh"
out="$(printf 'IN' | env -u BASH_GATES_FRESH BASH_GATES_DIR="$ST" "$BIN")"
assert_eq "fallback:IN" "$out" "9. source が新しければ未読の stdin ごと退避側へ exec"
out="$(printf '{}' | BASH_GATES_DIR="$ST" "$BIN")"
assert_eq 0 "$(grep -c fallback <<<"$out")" "9. FRESH があれば退避側へ行かない"

printf '結果: pass=%d fail=%d\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
