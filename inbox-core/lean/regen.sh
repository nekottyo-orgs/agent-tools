#!/usr/bin/env bash
# 目的: inbox-core/src/lib.rs から Charon → Aeneas で Lean を生成し、lean/ の証明を build して合否を機械で判定する
# 安全設計 (why):
#   - 作業はすべて mktemp -d の一時ディレクトリで行い、worktree に .lake / target / .llbc を作らない
#   - nightly の toolchain は AENEAS_SPIKE の env.sh だけで使い、mise の rust symlink を触らない (前後で比べて変われば exit 1)
#   - Aeneas / Lean が無い環境 (CI など) では「未実行」とだけ出して exit 0 で終わる。人の指示がある。
#     shell-style の「依存欠落は exit 2」より優先する。未実行の出力には成功と読める語を入れない
#   - 生成物 (InboxCore/InboxCore.lean) と lib.rs.sha256 が commit 済みと食い違えば exit 1 にして、証明の古びを気づかせる
# 使い方: bash regen.sh [--update]   (--update: 生成物と lib.rs.sha256 を上書きする)
#   AENEAS_SPIKE: spike の場所 (既定 $HOME/.local/share/aeneas-spike)
# 終了コード: 0 全部通った / 未実行 / 1 生成物か sha256 の食い違い・build か判定の失敗・symlink の変化 / 2 引数の誤り
set -euo pipefail

USAGE=$(cat <<'EOF'
使い方: bash regen.sh [--update]
EOF
)
update=0
while (($#)); do
  case "$1" in
  --update) update=1 ;;
  -h | --help) echo "$USAGE"; exit 0 ;;
  *) echo "$USAGE" >&2; exit 2 ;;
  esac
  shift
done

HERE="$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB="$HERE/../src/lib.rs"
SPIKE="${AENEAS_SPIKE:-$HOME/.local/share/aeneas-spike}"
MISE_RUST="$HOME/.local/share/mise/installs/rust/1.98.0"

if [ ! -f "$SPIKE/env.sh" ]; then
  echo "未実行: $SPIKE/env.sh が無い (Aeneas の spike が無い)"
  exit 0
fi
# shellcheck source=/dev/null
source "$SPIKE/env.sh"
for c in charon aeneas lake; do
  command -v "$c" >/dev/null 2>&1 || { echo "未実行: $c が無い"; exit 0; }
done

sha() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }

before="$(readlink "$MISE_RUST" || true)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export CARGO_TARGET_DIR="$TMP/target"

echo "charon: $(charon version 2>&1 | head -n 1 || true)"
echo "aeneas: $(aeneas -version 2>&1 | head -n 1 || true)"
# lean は elan 経由で、lean-toolchain のあるディレクトリでないと version が決まらない
echo "lean: $(cd "$HERE" && lean --version 2>&1 | head -n 1 || true)"

# 生成
mkdir -p "$TMP/crate/src"
cp "$LIB" "$TMP/crate/src/lib.rs"
printf '[package]\nname = "inbox_core"\nversion = "0.1.0"\nedition = "2021"\n\n[lib]\nname = "inbox_core"\n' >"$TMP/crate/Cargo.toml"
(cd "$TMP/crate" && charon cargo --preset=aeneas) >"$TMP/charon.log" 2>&1 || { tail -n 30 "$TMP/charon.log"; echo "判定: charon が失敗"; exit 1; }
aeneas -backend lean -dest "$TMP/InboxCore" "$TMP/crate/inbox_core.llbc" >"$TMP/aeneas.log" 2>&1 || { tail -n 30 "$TMP/aeneas.log"; echo "判定: aeneas が失敗"; exit 1; }

if ! diff -u "$HERE/InboxCore/InboxCore.lean" "$TMP/InboxCore/InboxCore.lean" >"$TMP/gen.diff"; then
  if [ "$update" -eq 1 ]; then
    cp "$TMP/InboxCore/InboxCore.lean" "$HERE/InboxCore/InboxCore.lean"
    echo "生成物: InboxCore/InboxCore.lean を更新した"
  else
    cat "$TMP/gen.diff"
    echo "判定: 生成物が commit 済みと違う。--update で更新"
    exit 1
  fi
else
  echo "生成物: commit 済みの InboxCore/InboxCore.lean と同一"
fi

# build (証明一式は生成物を含めて一時側へ複製する)
mkdir -p "$TMP/proofs"
cp -R "$HERE/Handoff" "$HERE/InboxCore" "$HERE/lean-toolchain" "$TMP/proofs/"
cat >"$TMP/proofs/lakefile.toml" <<EOF
name = "rust_core"
defaultTargets = ["Handoff", "InboxCore"]
packagesDir = "$SPIKE/lean-inbox/.lake/packages"

[[require]]
name = "aeneas"
path = "$SPIKE/aeneas/backends/lean"

[[lean_lib]]
name = "Handoff"
globs = ["Handoff.+"]

[[lean_lib]]
name = "InboxCore"
globs = ["InboxCore.+"]
EOF
rc=0
(cd "$TMP/proofs" && lake build) >"$TMP/build.log" 2>&1 || rc=$?
tail -n 15 "$TMP/build.log"

fail=0
if [ "$rc" -eq 0 ]; then echo "判定 (a): lake build は exit 0"; else echo "判定 (a): lake build が exit $rc で失敗"; fail=1; fi
if grep -Eq "declaration uses ['\`]sorry|sorryAx" "$TMP/build.log"; then echo "判定 (b): sorry が残っている"; fail=1; else echo "判定 (b): sorry なし"; fi
want="$(cat "$TMP"/proofs/Handoff/*.lean "$TMP"/proofs/InboxCore/*.lean | grep -c '#print axioms' || true)"
got="$(grep -c 'depends on axioms' "$TMP/build.log" || true)"
bad="$(grep 'depends on axioms' "$TMP/build.log" | grep -Evc '\[(propext|Classical\.choice|Quot\.sound)(, (propext|Classical\.choice|Quot\.sound))*\]$' || true)"
if [ "$want" -gt 0 ] && [ "$got" -eq "$want" ] && [ "$bad" -eq 0 ]; then
  echo "判定 (c): axioms $got 行 (#print axioms $want 個) は propext / Classical.choice / Quot.sound のみ"
else
  echo "判定 (c): axioms の行数 $got が #print axioms の数 $want と合わないか、許可外の axiom がある (許可外 $bad 行)"
  fail=1
fi
echo "定理の数: $(cat "$TMP"/proofs/Handoff/*.lean "$TMP"/proofs/InboxCore/Proofs.lean "$TMP"/proofs/InboxCore/Effects.lean "$TMP"/proofs/InboxCore/Stalled.lean | grep -Ec '^(theorem|lemma) ' || true)"

after="$(readlink "$MISE_RUST" || true)"
if [ "$before" != "$after" ]; then echo "判定: mise rust 1.98.0 の symlink が変わった (前 $before / 後 $after)"; exit 1; fi
[ "$fail" -eq 0 ] || { echo "判定: 失敗"; exit 1; }

# lib.rs の記録。証明したのは冒頭で一時側へ写した lib.rs なので、その写しの sha256 を残す
h="$(sha "$TMP/crate/src/lib.rs")"
if [ "$(sha "$LIB")" != "$h" ]; then
  echo "判定: lib.rs が検証中に変わった。再実行が必要"
  exit 1
fi
echo "lib.rs sha256: $h"
if [ "$update" -eq 1 ]; then
  echo "$h" >"$HERE/lib.rs.sha256"
  echo "lib.rs.sha256 を更新した"
elif [ "$(cat "$HERE/lib.rs.sha256" 2>/dev/null)" != "$h" ]; then
  echo "判定: lib.rs.sha256 が古い。--update で更新"
  exit 1
fi
echo "判定: 全部通った"
