---
name: verify-herdr-timeline
description: "macOS arm64 の herdr-timeline CLI を実機検証する。配布物の SHA-256 を照合し、隔離 Herdr で実 Codex に送った指示と pane、セッション ID、rollout、Timeline の本文を確認する。Timeline の変更時と Mac への反映後に使う。"
---

# Timeline で Codex の指示を確認する

[機能マップ](features/README.md) から検証する操作を選ぶ。最初に通した経路は、公開版と候補ビルドそれぞれで行った [Codex の指示表示](features/codex-history.md)。

## 起動

Apple Silicon Mac で実行する。必要なものは `herdr`、`codex`、Python 3.9 以上と、選んだバイナリ。初回は Herdr 0.9.5 と Codex 0.171.0 の組み合わせで実証した。公開物の取得には `gh`、候補版のビルドには Cargo を使う。

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

公開版は確認したいタグを指定する。バイナリと同じリリースのチェックサムを取得し、補助スクリプトが SHA-256 を照合する。公開されていなければ公開版を未確認と記録する。

```bash
(
  release_tag=herdr-timeline/v0.1.4
  evidence_root="$HOME/.local/state/herdr-timeline-verification"
  mkdir -p "$evidence_root" || exit
  run_id=$(uuidgen) || exit
  release_evidence="$evidence_root/$run_id-release"
  mkdir -m 700 "$release_evidence" || exit
  printf '%s\n' "$release_evidence"
  exec > "$release_evidence/run.log" 2>&1 || exit
  release_dir=$(mktemp -d /private/tmp/tl-release.XXXXXX) || exit
  trap '
    status=$?
    rm -rf -- "$release_dir" || {
      printf "Failed to remove release download: %s\n" "$release_dir" >&2
      if [ "$status" -eq 0 ]; then status=1; fi
    }
    exit "$status"
  ' EXIT
  asset=herdr-timeline-aarch64-apple-darwin
  gh release download "$release_tag" --repo nekottyo/agent-tools \
    --pattern "$asset*" --dir "$release_dir" &&
    gh release view "$release_tag" --repo nekottyo/agent-tools --json tagName,url,assets \
      > "$release_evidence/release.json" &&
    cp "$release_dir/$asset.sha256" "$release_evidence/" &&
    chmod +x "$release_dir/$asset" &&
    .claude/skills/verify-herdr-timeline/scripts/codex-history.py \
      "$release_dir/$asset" "$release_evidence/runner" \
      --release "$release_tag" "$release_evidence/$asset.sha256"
)
```

候補版は確認したいコミットのクリーンな checkout で実行する。毎回、新しい target ディレクトリにビルドし、成功して checkout の SHA が一致した場合に検証を起動する。

```bash
(
  evidence_root="$HOME/.local/state/herdr-timeline-verification"
  mkdir -p "$evidence_root" || exit
  run_id=$(uuidgen) || exit
  candidate_evidence="$evidence_root/$run_id-candidate"
  mkdir -m 700 "$candidate_evidence" || exit
  candidate_target=
  trap '
    run_exit=$?
    final_exit=$run_exit
    cleanup_exit=null
    if [ -n "$candidate_target" ]; then
      cleanup_exit=0
      rm -rf -- "$candidate_target" || cleanup_exit=$?
      if [ "$cleanup_exit" -ne 0 ]; then
        printf "Failed to remove candidate target: %s\n" "$candidate_target" >&2
        if [ "$final_exit" -eq 0 ]; then final_exit=1; fi
      fi
    fi
    result=failed
    if [ "$final_exit" -eq 0 ]; then result=passed; fi
    if ! (
      printf "{\"status\":\"%s\",\"exit\":%s,\"run_exit\":%s,\"cleanup_exit\":%s}\n" \
        "$result" "$final_exit" "$run_exit" "$cleanup_exit" > "$candidate_evidence/.result.json.tmp" &&
        mv "$candidate_evidence/.result.json.tmp" "$candidate_evidence/result.json"
    ); then
      printf "Failed to save candidate result: %s\n" "$candidate_evidence" >&2
      if [ "$final_exit" -eq 0 ]; then final_exit=1; fi
    fi
    exit "$final_exit"
  ' EXIT
  printf '%s\n' "$candidate_evidence"
  exec > "$candidate_evidence/run.log" 2>&1 || exit
  candidate_status=$(git status --porcelain) || exit
  test -z "$candidate_status" || exit
  candidate_commit=$(git rev-parse HEAD) || exit
  candidate_target=$(mktemp -d /private/tmp/tl-candidate.XXXXXX) || exit
  cargo build --locked -p herdr-timeline --target-dir "$candidate_target" &&
    test "$(git rev-parse HEAD)" = "$candidate_commit" &&
    .claude/skills/verify-herdr-timeline/scripts/codex-history.py \
      "$candidate_target/debug/herdr-timeline" "$candidate_evidence/runner" --commit "$candidate_commit"
)
```

各実行は、接続のない socket の警告、起動直後の正常な空一覧、送信前の履歴なしを区別した後、`herdr agent prompt` で生成した検証本文を送る。Codex の独立 backend を `--no-daemon` で起動し、Herdr が一時環境に登録した SessionStart だけを確認する。Codex にフックの信頼画面が出た場合は、一時環境でその 1 件だけが信頼待ちであることを確認してから、画面上で信頼を選ぶ。

合格条件は `herdr pane get` の対象 pane と native ID、rollout ファイル名と先頭の ID、完了済み UserMessage と `--timeline` の指示本文の一致。本文の表示は 1 回を要求する。送信や一覧取得が成功しても、native ID と履歴が揃わなければその経路は未確認と記録する。

## 証跡

候補版は、表示された証跡ディレクトリの `result.json` を最初に読む。`status: passed` と `exit: 0` が揃えば、新規 target の削除を含めて合格。`run_exit` はビルドと検証の終了コード、`cleanup_exit` は target 削除の終了コードで、target 作成前に止まった場合は `null`。失敗時は同じディレクトリの `run.log` でビルド出力と削除エラーを確認する。`result.json` が無ければ完了は未確認。

公開版の証跡には、配布情報の `release.json`、取得したチェックサム、実行出力の `run.log` が残る。公開版の合格にはコードブロックの終了コード 0 を確認する。

補助スクリプトの証跡は、公開版と候補版のどちらも `runner/` 配下にある。`commands.jsonl` に版、SHA、コマンド、標準出力・標準エラー・終了コード、hook の登録、pane・session・rollout・本文、停止結果が残る。コマンドのタイムアウトや起動失敗時も、対象コマンドと取得済みの出力が残る。待機の期限切れなど、補助スクリプトが失敗して終了した場合は、最後の行の `status: failed` と `reason` を確認する。`herdr.log` で隔離ソケットと起動を確認できる。`commands.jsonl` の最後の `status: passed` と `temporary_state_removed: true` は、補助スクリプトが隔離環境の片付けまで完了した印。初回試走は TUI のフォーカス、Claude Code の履歴、実アカウントや実モデルを対象としていない。

## 後片付け

スクリプトは自分で起動した Herdr のソケットだけに `herdr server stop` を送り、停止とソケット消失を確認してから模擬モデルと一時データを片付ける。異常終了時は `herdr.log` の socket を確認し、自分の試走のものと一致する場合に限り `HERDR_SOCKET_PATH=<その絶対パス> herdr server stop` で停止する。公開版と候補版の実行ブロックは終了時に、その回に作成したダウンロードまたは target のディレクトリを削除する。証跡ディレクトリは残す。

入口や合格条件が変わった日は `/maintain-verification-skill` で機能マップを照合する。
