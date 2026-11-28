#!/usr/bin/env bash
# ci-gate の集約ルールを表す Alloy モデル (.github/scripts/ci-gate/ci-gate.als) を検査する。
# 実行: repo root から `bash .github/scripts/ci-gate/check-model.sh`
# 依存: java / curl / sha256sum (無ければ shasum)。bash 3.2 でも動く範囲だけを使う。
#
# Alloy は 6.2.0 に固定する。配布 jar は AlloyTools の公式 release から取得し、
# 取得したファイルの sha256 が下の ALLOY_SHA256 と一致することを確認してから使う。
# URL やハッシュが変わったときは、この script の定数を更新する。
#
# 検査するコマンドと期待する結果 (ADR 0001 の「検証」節と同じ):
#   check Correspondence        -> UNSAT (scope 内に反例が無い)
#   check SixPositionsRejected  -> UNSAT (scope 内に反例が無い)
#   run   showPass              -> SAT   (成功する入力が存在する)
#   run   showSixPositions      -> SAT   (6 position の入力が存在する)
# 出力から結果を 1 行に特定できない場合や、結果が期待と違う場合は失敗にする (fail-close)。
set -uo pipefail

ALLOY_VERSION=6.2.0
ALLOY_URL="https://github.com/AlloyTools/org.alloytools.alloy/releases/download/v${ALLOY_VERSION}/org.alloytools.alloy.dist.jar"
# org.alloytools.alloy.dist.jar (v6.2.0) の sha256
ALLOY_SHA256="6b8c1cb5bc93bedfc7c61435c4e1ab6e688a242dc702a394628d9a9801edb78d"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODEL="$SCRIPT_DIR/ci-gate.als"

failures=0

sha256_of() { # $1=file
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d ' ' -f 1
  else
    echo "sha256 を計算できない (sha256sum も shasum も無い)" >&2
    return 1
  fi
}

fetch_alloy() { # 標準出力に jar の path を返す
  # CI では RUNNER_TEMP、手元では TMPDIR 配下に置く。既に同じハッシュの jar があれば再利用する
  cache_dir="${ALLOY_CACHE_DIR:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}/ci-gate-alloy}"
  jar="$cache_dir/alloy-$ALLOY_VERSION.jar"

  if [ -f "$jar" ]; then
    actual="$(sha256_of "$jar")" || return 1
    if [ "$actual" = "$ALLOY_SHA256" ]; then
      printf '%s\n' "$jar"
      return 0
    fi
    echo "キャッシュされた Alloy の sha256 が一致しないため取得し直す: $jar" >&2
  fi

  mkdir -p "$cache_dir" || return 1
  tmp="$jar.tmp.$$"
  rm -f "$tmp"
  if ! curl -fsSL --retry 3 --retry-delay 2 -o "$tmp" "$ALLOY_URL"; then
    echo "Alloy の取得に失敗: $ALLOY_URL" >&2
    rm -f "$tmp"
    return 1
  fi

  actual="$(sha256_of "$tmp")" || {
    rm -f "$tmp"
    return 1
  }
  if [ "$actual" != "$ALLOY_SHA256" ]; then
    echo "Alloy の sha256 が一致しない: 期待 $ALLOY_SHA256 / 実際 $actual" >&2
    rm -f "$tmp"
    return 1
  fi

  mv "$tmp" "$jar" || {
    rm -f "$tmp"
    return 1
  }
  printf '%s\n' "$jar"
}

run_case() { # $1=kind (check/run) $2=name $3=期待する結果 (UNSAT/SAT)
  local kind=$1 name=$2 expected=$3
  local out="$work/$name.log" status found line token

  rm -rf "$work/out-$name"
  # Alloy は結果行を stderr に書くため、stdout と stderr をまとめて 1 つの log にする
  if ! java -jar "$jar" exec -f -c "$name" -t text -o "$work/out-$name" "$MODEL" \
    >"$out" 2>&1; then
    echo "NG   $kind $name: Alloy の実行に失敗した" >&2
    cat "$out" >&2
    failures=$((failures + 1))
    return
  fi

  # 結果行は `00. check Correspondence ... UNSAT` や `00. run showPass ... SAT` の形。
  # 対象のコマンド名を含む行を探し、最後の語を結果として読む。行が 1 つに定まらない、
  # または最後の語が UNSAT / SAT でない場合は判定不能として失敗にする。
  found=0
  status=unknown
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line%$'\r'}"
    case "$line" in
      *"$kind"*"$name"*) ;;
      *) continue ;;
    esac
    line="${line%"${line##*[![:space:]]}"}"
    token="${line##*[[:space:]]}"
    case "$token" in
      UNSAT|SAT) ;;
      *) token=unknown ;;
    esac
    found=$((found + 1))
    status=$token
  done <"$out"

  if [ "$found" -ne 1 ]; then
    echo "NG   $kind $name: 結果行を 1 行に特定できない (該当 $found 行)" >&2
    cat "$out" >&2
    failures=$((failures + 1))
    return
  fi
  if [ "$status" != "$expected" ]; then
    echo "NG   $kind $name: 期待 $expected / 実際 $status" >&2
    cat "$out" >&2
    failures=$((failures + 1))
    return
  fi
  printf 'OK   %s %s: %s\n' "$kind" "$name" "$status"
}

if [ ! -f "$MODEL" ]; then
  echo "モデルが無い: $MODEL" >&2
  exit 1
fi
if ! command -v java >/dev/null 2>&1; then
  echo "java が見つからない (CI では setup-java で Temurin 21 を用意する)" >&2
  exit 1
fi
if ! jar="$(fetch_alloy)"; then
  exit 1
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/ci-gate-alloy.XXXXXX")" || exit 1
trap 'rm -rf "$work"' EXIT

java -version >&2

run_case check Correspondence UNSAT
run_case check SixPositionsRejected UNSAT
run_case run showPass SAT
run_case run showSixPositions SAT

if [ "$failures" -ne 0 ]; then
  printf 'Alloy モデル検査: %d 件失敗\n' "$failures" >&2
  exit 1
fi
echo "Alloy モデル検査: すべて期待どおり"
