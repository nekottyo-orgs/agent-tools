# herdr-timeline

herdr (端末の multiplexer) の pane を「要対応 / 作業中 / done・idle」の 3 段で一覧し、選んだ pane の Timeline (人の指示、他 pane からのメッセージ、PR、inbox の報告) を新しい順に出す端末 UI。DeepSeek Harness (DSH) の Timeline を参考にした。

- 要対応: blocked の pane と、inbox/new に未処理の報告がある run
- 作業中: working の pane。brief のゴールと step の進捗を要約に出す
- done / idle: workspace ごとに畳んで件数だけ出す
- 常駐の pane (Cue の回答を処理する専任 worker、watch 系の監視、watch-worker、review-submit) は既定で一覧に出さない。herdr の API に常駐の印が無いので、pane の label の完全一致で見分ける (一覧は `src/data.rs` の `RESIDENT_LABELS`)。監視 skill の dispatch が起こす PR review の pane (label が `<repo略称>-<PR番号>` / `<repo略称>-bundle-<PR番号>`) も外す。`feat-xxx-34` のような無関係な pane に当たらないよう、label の形と「run に属さない」(inbox の msg が帰属しない、orchestrator_pane でない) の両方で見分け、`--runs-dir` が無いときは外さない。ただし blocked の pane は人の対応待ちなので隠さない。`--show-resident` で全部出せる
- 状態の記号と色は herdr 本体の `status_indicators = "symbols"` と同じ (blocked は赤の `×`、working は黄の `◐`、done は青緑の `✓`、idle は緑の `○`、unknown は `·`)。色が出ない端末でも記号の形で区別できる。run の未処理の報告の行頭にある `▲` は pane の状態ではなく別の印なので、そのまま残している

## 読み取り範囲と安全設計

- 読むだけで書き込まない。例外は Enter の `herdr tab focus` だけで、pane には入力しない。サーバを立てない
- 読むもの
  - herdr の `pane list` / `tab list` / `workspace list` (session ごと)
  - Claude Code の transcript (`<projects>/<dir>/<session_id>.jsonl`)。全 pane の transcript は、ゴールを出すために前回読んだ位置から追記ぶんだけを読む (brief のパスと最初の時刻だけを拾う)。Timeline は開いた pane と `--timeline` の pane だけ、全体を読む。session_id は英数字と `. _ -` だけ、先頭が `.` でないものしか使わない
  - run の `run.md` と `inbox/new`・`inbox/cur` の `*.msg` (それ以外のディレクトリは見ない)。pane 行の進捗には、名前 (`<prio>-<micros>-<tag>-<from>-<rand>.msg`) の tag が `progress` のものだけを開く。Timeline を開いた pane には全部を開く
  - transcript の指示が指す brief (`brief-<name>.md`)
- 外部入力 (transcript と inbox の本文、pane の label と title) は、端末に出す前に CSI 列を丸ごと落とし、残りの制御文字を空白にする
- 読めないファイルや herdr の失敗では落ちない。イベント 0 件か、画面上部の警告 1 行になる
- inbox の報告は `from` (pane id)、`from_socket` (送り手の herdr の socket)、`sent_at` が transcript の開始以降、の 3 つが合うときだけその pane のものとみなす。transcript が空か壊れていて開始時刻が分からない pane には帰属させない。`from_socket` の無い旧形式は帰属させない (別 session の同じ pane id と区別できないため)

## 使い方

```
herdr-timeline [--once] [--interval N] [--width N] [--timeline PANE_ID] [--runs-dir DIR] [--show-resident] [-h|--help]
```

| オプション | 意味 |
| --- | --- |
| `--once` | 1 回描画して終わる |
| `--interval N` | 更新の間隔 (秒、既定 5、上限 86400) |
| `--width N` | 表示幅を固定する (既定は端末幅 - 1、最小 30、上限 1000) |
| `--timeline PANE_ID` | pane の Timeline を 1 回出して終わる (非対話)。`<session>/<pane>` でなく pane id だけを渡したときは、今の session の pane を優先し、無ければ他の session から探す |
| `--runs-dir DIR` | run のディレクトリ (`DIR/<run>/run.md`、`DIR/<run>/inbox/{new,cur}/*.msg`)。無ければ報告・進捗・未処理の報告を出さないだけで動く |
| `--show-resident` | 常駐の pane (label が `RESIDENT_LABELS` と完全一致) と dispatch の PR review の pane も一覧に出す。`--timeline` では常に出る |
| `-h`, `--help` | 説明を出す |

run のディレクトリに既定値は無い。取り込み側が `--runs-dir` か環境変数で渡す。

| 環境変数 | 意味 |
| --- | --- |
| `HERDR_TIMELINE_HERDR` | herdr コマンド (既定 `herdr`、空白区切りで分割) |
| `HERDR_TIMELINE_HERDR_DIR` | socket を探す場所 (既定 `~/.config/herdr`)。`herdr.sock` と `sessions/*/herdr.sock` が 2 つ以上あれば session ごとに取り、行末に `<session>/<pane>` を出す |
| `HERDR_TIMELINE_PROJECTS` | transcript の projects ディレクトリ (既定 `~/.claude/projects`) |
| `HERDR_TIMELINE_RUNS_DIR` | `--runs-dir` と同じ |
| `HERDR_SOCKET_PATH` | 今の session の socket |
| `NO_COLOR` / `FORCE_COLOR` | 色を消す / tty でなくても色を付ける |

### 終了コード

| コード | 意味 |
| --- | --- |
| 0 | 正常 (`-h`、`--once`・`--timeline` の描画、対話の終了) |
| 1 | `--timeline` の pane が見つからない |
| 2 | 引数の誤り (値の欠落、値が `-` で始まる、正の整数でない、`--interval` が 86400 を超える、`--width` が 1000 を超える、不明な引数) |

### キー操作

| キー | 動作 |
| --- | --- |
| `↑` `↓` / `j` `k` | 要対応と作業中の行を選ぶ |
| `Enter` | 選んだ行の tab へ移動する (`herdr tab focus`)。別 session の行では移らずに、移れない旨を出す |
| `t` | 選んだ行の Timeline を開閉する |
| `q` | 終了する (Ctrl-C でも端末を戻して終わる) |

## Timeline

- 種類は `指示` (人の指示。貼り付けは中身の最初の行)、`from` (`[herdr-message]` で届いた他 pane のメッセージ)、`PR` (URL の初出)、`報告` (inbox)
- 出さないもの: meta / sidechain / compact summary の行、`<` や `Stop hook feedback:` などで始まる自動の入力。判定は先頭の空白を除いてから行う
- 並びは (時刻, 源, 源の中の連番) の降順。同時刻なら inbox が先、同じ源なら transcript の行番号か msg のファイル名順で決まる
- 隣り合う同じ種類を 1 行にまとめて `×N` を付け、最大 8 行。残りは「他 N 件」

## brief の steps と [progress]

transcript の指示のうち、`/…/brief-<name>.md` の絶対パスを含む**最後の**ものを brief とみなす (出し直しに追随する)。brief の最初の `# ` 行 (先頭の「依頼: 」は除く) がゴールになる。

brief の先頭に frontmatter で step を書くと進捗を出せる。flow 形式の配列だけを読む。

```markdown
---
steps: [調査, 対応表, 設計案]
---
# 依頼: 対応表を作る
```

worker は inbox に `tag: progress` の msg を送り、本文の最初の行に済んだ step 名を `,` か `、` 区切りで**全部** (差分でなく) 書く。

```
[progress] 調査, 対応表
```

- 最後の brief を指示した時刻より前の progress は数えない。最新 (時刻、同時刻ならファイル名順) の 1 通だけを使う
- progress が 1 通も無ければ「進捗未報告」(0 step 済みとは区別する)
- brief に無い step 名は済みに数えず、「brief に無い step k 件」と件数だけ出す

## Lean の model

判定部 (並び順、まとめ、行の分類、帰属、session_id の検証、brief と進捗) は [`lean/HerdrTimeline.lean`](lean/HerdrTimeline.lean) にモデルと性質の証明がある。Rust (`src/core.rs`) は関数の名前と判定をモデルに揃え、一致は `src/core.rs` の `#[cfg(test)]` が同じ入力の例で確かめる。

証明が残っている (`sorry`) 定理は無い (`lake build` が通り、`sorry` は 0 件)。
