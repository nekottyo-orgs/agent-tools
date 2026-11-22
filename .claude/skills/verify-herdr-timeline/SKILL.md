---
name: verify-herdr-timeline
description: "macOS arm64 の herdr-timeline CLI を実機検証する。配布物の SHA-256 を照合し、隔離 Herdr で実 Codex に送った指示と pane、セッション ID、rollout、Timeline の本文を確認する。Timeline の変更時と Mac への反映後に使う。"
---

# Timeline で Codex の指示を確認する

[機能マップ](features/README.md) から検証する操作を選ぶ。最初に通した経路は、公開版と候補ビルドそれぞれで行った [Codex の指示表示](features/codex-history.md)。

## 起動

Apple Silicon Mac で実行する。必要なものは `herdr`、`codex`、Python 3.9 以上と、選んだバイナリ。初回は Herdr 0.9.5 と Codex 0.171.0 の組み合わせで実証した。公開物の取得には `gh`、候補版のビルドには Cargo を使う。

リポジトリルートで、公開版のバイナリと同じリリースのチェックサムを取得する。確認したい版に合わせてタグを指定する。公開されていなければ公開版を未確認と記録する。

```bash
release_tag=herdr-timeline/v0.1.4
release_dir=$(mktemp -d /private/tmp/tl-release.XXXXXX)
gh release download "$release_tag" --repo nekottyo/agent-tools \
  --pattern 'herdr-timeline-aarch64-apple-darwin*' --dir "$release_dir"
gh release view "$release_tag" --repo nekottyo/agent-tools --json tagName,url,assets > "$release_dir/release.json"
(cd "$release_dir" && shasum -a 256 -c herdr-timeline-aarch64-apple-darwin.sha256)
chmod +x "$release_dir/herdr-timeline-aarch64-apple-darwin"
```

候補版は確認したいコミットの checkout でビルドする。作業ツリーに未コミットの製品コードがあればその差分も記録する。

```bash
candidate_commit=$(git rev-parse HEAD)
cargo build --locked -p herdr-timeline
```

補助スクリプトが毎回、一時 HOME、Codex ディレクトリ、Herdr ソケット、空の作業ディレクトリを作る。実 Herdr の `server` を起動し、そのソケットで `pane list` が成功した時点で起動完了。モデル境界だけを認証不要の loopback HTTP に置き換える。共有 socket・共有 Codex backend・普段の hook 設定を使う試走は中止する。

## 読み取り点検

版更新時や失敗時は、次の読み取り専用コマンドで PATH、登録済み統合、インストール済みバイナリを確認する。

```bash
uname -m
command -v herdr codex herdr-timeline
herdr --version
codex --version
herdr integration status
file "$(command -v herdr-timeline)"
shasum -a 256 "$(command -v herdr-timeline)"
```

インストール済みバイナリを試す場合も、配布物のチェックサムとの一致を要求する。実行中の一覧が必要な場合は `herdr-timeline --json` の `warnings` と `panes` を確認し、実際の pane の本文を証跡へコピーしない。

## 操作

リポジトリルートで実行する。補助スクリプトは 127.0.0.1 の待受と実 Herdr の Unix socket を使う。シェルの実行環境にその権限が必要。公開版と候補版には別々の証跡ディレクトリを指定する。

```bash
evidence_root="$HOME/.local/state/herdr-timeline-verification"
mkdir -p "$evidence_root"
run_id=$(uuidgen)
.claude/skills/verify-herdr-timeline/scripts/codex-history.py \
  "$release_dir/herdr-timeline-aarch64-apple-darwin" "$evidence_root/$run_id-release" \
  --release "$release_tag" "$release_dir/herdr-timeline-aarch64-apple-darwin.sha256"
cp "$release_dir/release.json" "$evidence_root/$run_id-release/release.json"
run_id=$(uuidgen)
.claude/skills/verify-herdr-timeline/scripts/codex-history.py \
  target/debug/herdr-timeline "$evidence_root/$run_id-candidate" --commit "$candidate_commit"
```

各実行は、接続のない socket の警告、起動直後の正常な空一覧、送信前の履歴なしを区別した後、`herdr agent prompt` で生成した検証本文を送る。Codex の独立 backend を `--no-daemon` で起動し、Herdr が一時環境に登録した SessionStart だけを確認する。Codex にフックの信頼画面が出た場合は、一時環境でその 1 件だけが信頼待ちであることを確認してから、画面上で信頼を選ぶ。

合格条件は `herdr pane get` の対象 pane と native ID、rollout ファイル名と先頭の ID、完了済み UserMessage と `--timeline` の指示本文の一致。本文の表示は 1 回を要求する。送信や一覧取得が成功しても、native ID と履歴が揃わなければその経路は未確認と記録する。

## 証跡

`commands.jsonl` に版、SHA、コマンド、標準出力・標準エラー・終了コード、hook の登録、pane・session・rollout・本文、停止結果が残る。`herdr.log` で隔離ソケットと起動を確認できる。最後の `status: passed` と `temporary_state_removed: true` は片付けも完了した印。失敗時は最後の操作、エラー、未確認の項目を記録する。初回試走は TUI のフォーカス、Claude Code の履歴、実アカウントや実モデルを対象としていない。

## 後片付け

スクリプトは自分で起動した Herdr のソケットだけに `herdr server stop` を送り、停止とソケット消失を確認してから模擬モデルと一時データを片付ける。異常終了時は `herdr.log` の socket を確認し、自分の試走のものと一致する場合に限り `HERDR_SOCKET_PATH=<その絶対パス> herdr server stop` で停止する。証跡ディレクトリは残す。

入口や合格条件が変わった日は `/maintain-verification-skill` で機能マップを照合する。
