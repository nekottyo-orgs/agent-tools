#!/usr/bin/env bash
# 目的: inbox-core の CI 用に固定した Charon / Aeneas / Lean を一時ディレクトリへ置く
# 安全設計: 集めた upstream の tarball は SHA-256 が一致したものだけを展開する
# 使い方: AENEAS_SPIKE=<ディレクトリ> bash inbox-core/lean/setup-ci.sh (Linux x86_64)
# 終了コード: 0 導入完了 / 非ゼロ 取得か検証に失敗
set -euo pipefail

: "${AENEAS_SPIKE:?AENEAS_SPIKE is required}"
proof_dir="$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$AENEAS_SPIKE"
cd "$AENEAS_SPIKE"

download() {
  local url="$1" digest="$2" name="${1##*/}"
  curl -fsSL --retry 3 -o "$name" "$url"
  printf '%s  %s\n' "$digest" "$name" | sha256sum -c -
}

release=https://github.com/AeneasVerif/aeneas/releases/download/nightly-2026.11.19-8f4466d
download "$release/aeneas-linux-x86_64.tar.gz" 00d4e95028bc5e27c0b4c3195079a421a9ac68f3e3d92c5e6260871e827d0be6
mkdir -p aeneas lean-inbox/.lake
tar -xzf aeneas-linux-x86_64.tar.gz -C aeneas

# 固定した配布物の manifest を使い、Lake に先立って独立した Git 取得を進める。
fetch_package() {
  local pkg="$1" url="$2" rev="$3"
  git init --quiet "$pkg" &&
    git -C "$pkg" remote add origin "$url" &&
    git -C "$pkg" fetch --quiet --no-tags --depth 1 origin "$rev" &&
    git -C "$pkg" checkout --quiet --detach "$rev"
}

prefetch_packages() (
  staging="$(mktemp -d "${packages}.XXXXXX")"
  trap 'rm -rf -- "$staging"' EXIT
  manifest_rows="$(jq -er '.packages[] |
    if .type == "git" and (.name | test("^[a-zA-Z0-9_-]+$")) and
       (.url | startswith("https://github.com/")) and (.rev | test("^[0-9a-f]{40}$"))
    then [.name, .url, .rev] | @tsv
    else error("unsupported entry in Aeneas package manifest") end' \
    aeneas/backends/lean/lake-manifest.json)"
  fetches=()
  while IFS=$'\t' read -r name url rev; do
    fetch_package "$staging/$name" "$url" "$rev" &
    fetches+=("$!")
  done <<<"$manifest_rows"
  fetch_rc=0
  for fetch_pid in "${fetches[@]}"; do wait "$fetch_pid" || fetch_rc=$?; done
  [ "$fetch_rc" -eq 0 ] || exit "$fetch_rc"
  # 途中で失敗した checkout を次回の導入が再利用しないよう、全件成功後に公開する。
  mv -T -- "$staging" "$packages"
)

packages=lean-inbox/.lake/packages
package_setup=
if [ ! -d "$packages" ]; then
  prefetch_packages &
  package_setup=$!
fi

download "$release/lean-build-aeneas-x86_64-unknown-linux-gnu.tar.gz" 84896c4bc46d93c87e0e4cfb09b602d0e94b60b7fb9373871d70244b9a49d525
download https://github.com/leanprover/elan/releases/download/v4.2.4/elan-x86_64-unknown-linux-gnu.tar.gz 42b94d4244e8353142c456ec0e4ca6528fd898a6c604d4059f494e706e431f63
mkdir -p aeneas/backends/lean/.lake/build
tar -xzf lean-build-aeneas-x86_64-unknown-linux-gnu.tar.gz -C aeneas/backends/lean/.lake/build
tar -xzf elan-x86_64-unknown-linux-gnu.tar.gz

# Charon はこの rustc の内部 API を使う。stable は既存の Rust テスト用に保持する。
rust_toolchain=nightly-2026-09-17
export ELAN_HOME="$PWD/elan"
lean_toolchain="$(cat "$proof_dir/lean-toolchain")"
if [ "$lean_toolchain" != "$(cat aeneas/backends/lean/lean-toolchain)" ]; then
  echo "Pinned Lean toolchain does not match the Aeneas release" >&2
  exit 1
fi

# 異なるディレクトリに導入するため、nightly Rust と Lean は並行して展開できる。
rustup toolchain install "$rust_toolchain" --profile minimal --component rustc-dev,rust-src &
rust_install=$!
lean_rc=0
./elan-init -y --no-modify-path --default-toolchain "$lean_toolchain" &&
  "$ELAN_HOME/bin/elan" toolchain install "$lean_toolchain" || lean_rc=$?
wait "$rust_install"
[ "$lean_rc" -eq 0 ] || exit "$lean_rc"
if [ -n "$package_setup" ]; then wait "$package_setup"; fi
# PATH は regen.sh が env.sh を読む時点で展開する。
# shellcheck disable=SC2016
printf 'export ELAN_HOME=%q\nexport RUSTUP_TOOLCHAIN=%q\nexport PATH=%q:$PATH\n' \
  "$ELAN_HOME" "$rust_toolchain" "$PWD/aeneas:$ELAN_HOME/bin" >env.sh
