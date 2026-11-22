# herdr-timeline

herdr (端末の multiplexer) の pane を「要対応 / 作業中 / idle・unknown」の 3 段で一覧し、選んだ pane の Timeline (人の指示、他 pane からのメッセージ、PR、inbox の報告) を新しい順に出す端末 UI。DeepSeek Harness (DSH) の Timeline を参考にした。

- 要対応: blocked の pane と、inbox/new に未処理の報告がある run。窓口の稼働・不在・状態未確認・終了済みを区別し、稼働中以外の行には run 名と未処理件数を表示する
- 作業中: working の pane。brief のゴールと step の進捗を要約に出す
- idle / unknown は workspace ごとに件数を表示。集計対象があるときだけ見出しを表示
- done の pane は対話表示と `--once` の一覧から除外。pane の履歴は `--timeline PANE_ID`、状態は `--json`、完了した依頼は Cue の完了一覧で確認
- 常駐の pane (Cue の回答を処理する専任 worker、watch 系の監視、watch-worker、review-submit) は既定で一覧に出さない。herdr の API に常駐の印が無いので、pane の label の完全一致で見分ける (一覧は `src/data.rs` の `RESIDENT_LABELS`)。監視 skill の dispatch が起こす PR review の pane (label が `<repo略称>-<PR番号>` / `<repo略称>-bundle-<PR番号>`) も外す。`feat-xxx-34` のような無関係な pane に当たらないよう、label の形と「run に属さない」(inbox の msg が帰属しない、orchestrator_pane でない) の両方で見分け、`--runs-dir` が無いときは外さない。ただし blocked の pane は人の対応待ちなので隠さない。起動直後で最初の報告がまだ無い worker は、label が dispatch の形だと報告が付くまで隠れうる (窓口の orchestrator_pane は外れない)。`--show-resident` で常駐・dispatch による除外を解除できる
- 状態の記号と色は herdr 本体の `status_indicators = "symbols"` と同じ (blocked は赤の `×`、working は黄の `◐`、idle は緑の `○`、unknown は `·`)。色が出ない端末でも記号の形で区別できる。run の未処理の報告は `▲` で表示

## 読み取り範囲と安全設計

- 読むだけで書き込まない。例外は Enter の `herdr tab focus` だけで、pane には入力しない。サーバを立てない
- 読むもの
  - herdr の `pane list` / `tab list` / `workspace list` (session ごと)
  - Claude Code の transcript (`<projects>/<dir>/<session_id>.jsonl`)。全 pane の transcript は、ゴールを出すために前回読んだ位置から追記ぶんだけを読む (brief のパスと、最初と最後の入力の時刻だけを拾う。queue-operation (enqueue / dequeue / remove / popAll) も読み、キュー待ちの入力の件数と時刻を数える)。Timeline は選んだ pane と `--timeline` の pane だけ、全体を読む。session_id は英数字と `. _ -` だけ、先頭が `.` でないものしか使わない
  - Codex の rollout (`$CODEX_HOME/sessions/YYYY/MM/DD/rollout-…-<UUID>.jsonl`、`CODEX_HOME` の既定は `~/.codex`)。Herdr の Codex 統合が現在の pane に報告した ID と、ファイル名・先頭の `session_meta.payload.id` が一致し、候補が 1 件のときに読む。Codex 0.171.0 の完了済み `UserMessage` を指示・経過・brief の起点に使う。履歴の ID が未報告なら「transcript が見つからない」と表示する
  - run の `run.md` と `inbox/new`・`inbox/cur` の `*.msg` (それ以外のディレクトリは見ない)。pane 行の進捗には、名前 (`<prio>-<micros>-<tag>-<from>-<rand>.msg`) の tag が `progress` のものだけを開く。Timeline を出す pane には全部を開く
  - transcript の指示が指す brief (`brief-<name>.md`)
- 外部入力 (transcript と inbox の本文、pane の label と title) は、端末に出す前に CSI 列を丸ごと落とし、残りの制御文字を空白にする
- 読めないファイルや herdr の失敗では落ちない。イベント 0 件か、画面上部の警告 1 行になる
- inbox の報告は `from` (pane id)、`from_socket` (送り手の herdr の socket)、`sent_at` が transcript の開始以降、の 3 つが合うときだけその pane のものとみなす。transcript が空か壊れていて開始時刻が分からない pane には帰属させない。`from_socket` の無い旧形式は帰属させない (別 session の同じ pane id と区別できないため)

Codex のキュー件数は `--json` の `queue: null` で示す。Claude Code の `pr-link` と同じ記録が Codex の rollout に無いため、Codex 上の PR 操作は inbox の報告で確認できる。

## インストール

Release の asset (`herdr-timeline-<target>` と `.sha256`) を取って検証し、実行権限を付けて置く。target は `x86_64-unknown-linux-gnu` か `aarch64-apple-darwin`。tag は `herdr-timeline/v<version>`。

```bash
gh release download herdr-timeline/v<version> --repo nekottyo/agent-tools --pattern 'herdr-timeline-<target>*'
shasum -a 256 -c herdr-timeline-<target>.sha256 \
  && chmod +x herdr-timeline-<target> \
  && mkdir -p "$HOME/.cargo/bin" \
  && mv herdr-timeline-<target> "$HOME/.cargo/bin/herdr-timeline"
```

`$HOME/.cargo/bin` が PATH に無いときは、`export PATH="$HOME/.cargo/bin:$PATH"` を shell の設定に足すか、`"$HOME/.cargo/bin/herdr-timeline"` とフルパスで呼ぶ。

`cargo install --path herdr-timeline --locked` でも入れられる。

## 使い方

```
herdr-timeline [--once] [--interval N] [--width N] [--timeline PANE_ID] [--runs-dir DIR] [--show-resident] [--json] [-h|--help]
```

| オプション | 意味 |
| --- | --- |
| `--once` | 1 回描画して終わる |
| `--interval N` | 更新の間隔 (秒、既定 5、上限 86400) |
| `--width N` | 表示幅を固定する (既定は端末幅 - 1、最小 30、上限 1000) |
| `--timeline PANE_ID` | pane の Timeline を 1 回出して終わる (非対話)。`<session>/<pane>` でなく pane id だけを渡したときは、今の session の pane を優先し、無ければ他の session から探す |
| `--runs-dir DIR` | run のディレクトリ (`DIR/<run>/run.md`、`DIR/<run>/inbox/{new,cur}/*.msg`)。無ければ報告・進捗・未処理の報告を出さないだけで動く |
| `--show-resident` | 常駐の pane (label が `RESIDENT_LABELS` と完全一致) と dispatch の PR review の pane を一覧の対象に含める。`--timeline` では常に出る |
| `--json` | 全 pane の状態を JSON 1 つで出して終わる (`--once` を含む)。`--timeline` とは併用できない。常駐・dispatch の pane も `hidden` を付けて全部出す (後述) |
| `-h`, `--help` | 説明を出す |

run のディレクトリに既定値は無い。取り込み側が `--runs-dir` か環境変数で渡す。

| 環境変数 | 意味 |
| --- | --- |
| `HERDR_TIMELINE_HERDR` | herdr コマンド (既定 `herdr`、空白区切りで分割) |
| `HERDR_TIMELINE_HERDR_DIR` | socket を探す場所 (既定 `~/.config/herdr`)。`herdr.sock` と `sessions/*/herdr.sock` が 2 つ以上あれば session ごとに取り、行末に `<session>/<pane>` を出す |
| `HERDR_TIMELINE_PROJECTS` | transcript の projects ディレクトリ (既定 `~/.claude/projects`) |
| `HERDR_TIMELINE_RUNS_DIR` | `--runs-dir` と同じ |
| `HERDR_SOCKET_PATH` | 今の session の socket |
| `HERDR_TIMELINE_CALLER` | 呼び出し元の pane id。起動時に、その pane が要対応か作業中ならその行を選ぶ。空や不正な値は無視する。表示の初期値だけ |
| `NO_COLOR` / `FORCE_COLOR` | 色を消す / tty でなくても色を付ける |

### 終了コード

| コード | 意味 |
| --- | --- |
| 0 | 正常 (`-h`、`--once`・`--json`・`--timeline` の描画、対話の終了) |
| 1 | `--timeline` の pane が見つからない |
| 2 | 引数の誤り (値の欠落、値が `-` で始まる、正の整数でない、`--interval` が 86400 を超える、`--width` が 1000 を超える、不明な引数、`--json` と `--timeline` の併用) |

### キー操作

| キー | 動作 |
| --- | --- |
| `↑` `↓` / `j` `k` | 要対応と作業中の行を選ぶ |
| `Enter` | 選んだ行の tab へ移動する (`herdr tab focus`)。別 session の行では移らずに、移れない旨を出す |
| `Esc` / `q` | 終了する (Ctrl-C でも端末を戻して終わる)。Esc は単独で押したときだけ (矢印などの列は終了にならない) |

## JSON 出力

`--json` は stdout に JSON のトップレベル object 1 つだけを出す (末尾は改行 1 つ)。判定は画面と同じ関数を通す。

- `panes`: pane の配列。常駐・dispatch の pane も含め、画面で隠すかどうかは `hidden` で示す
- `warnings`: 警告の配列。一覧取得 (pane、workspace、tab) の失敗や、`panes` が配列でない場合、有効な `pane_id` のない要素がある場合に診断を含む。有効な ID のある行は引き続き表示し、`panes: []` は正常な空一覧として扱う
- `watchers`: 常駐 watcher 4 本 (`watch-pane-turns` / `watch-my-pr-reviews` / `watch-other-pr-reviews` / `watch-merged-prs`、この順) の配列。各要素は `name`、`running`、`pane`、`state_dir`、`last_activity`。`--once` の画面には出さない
  - `running`: watcher のスクリプトを直接実行しているか、`bash` のスクリプト引数として実行していれば true。比較するのはその位置のファイル名だけで、名前は完全一致とする。停止中のプロセスやゾンビは除く
  - Bash では `bash script`、`bash -- script`、`bash -e script`、`bash -eux script`、`bash -o pipefail script` などを判定する。スクリプトの前で読み飛ばすのは `abBCeEfhHkmpPTuvx` の短いフラグ (`-` / `+`、連結可) と、名前を 1 語取る `-o` / `+o` / `-O` / `+O`。`-c` (コマンド文字列) や `-s` (標準入力) を含む起動は対象外
  - `ps` は引数を空白つなぎで返す。たとえば `bash /a b/watch-pane-turns.sh` は、空白を含む 1 つのスクリプトパスと、`/a` に `b/watch-pane-turns.sh` を渡した起動を区別できない。判定は空白を含まない実行パスを前提とする
  - lock を持つ 3 本は `<state_dir>/resident.lock/pid` (merged は `merged-resident.lock/pid`) の PID を `ps` で照合する。PID ファイルが無い・空・数字でない場合や `ps` が失敗した場合は false。`watch-pane-turns` は lock が無いため、`ps` のプロセス一覧で照合する (herdr-timeline 自身の行は除く)。`ps` は `HERDR_TIMELINE_PS_BIN` で差し替えられる
  - `pane`: label が `name` と完全一致する pane の `pane_id` (複数なら先頭、無ければ null)
  - `state_dir`: 各 watcher の env (`WATCH_PANE_TURNS_STATE_DIR` / `WATCH_PR_REVIEWS_STATE_DIR` / `WATCH_REVIEW_REQUESTS_STATE_DIR`) か、既定の `~/.agents/state/…` を展開した値。`last_activity` はその直下のエントリと、直下のディレクトリの 1 段下のエントリ (lock ディレクトリを除く、2 階層まで) の mtime の最大 (epoch 秒)。空の env は既定扱い。無い・空なら null

| 項目 | 型 | 意味 |
| --- | --- | --- |
| `server` | string か null | socket が複数のときの session 名。1 つだけなら null |
| `pane_id` | string | session 修飾なしの pane id |
| `key` | string | server があれば `<server>/<pane_id>`、無ければ `pane_id` (画面の行末の tag と同じ) |
| `label` | string | herdr の pane label (制御文字は落とし済み) |
| `status` | string | `blocked` / `working` / `idle` / `done` / `unknown` |
| `working_secs` | integer か null | working のとき、transcript の最後の入力からの経過秒。起点が transcript から取れない (画面の `≥`) ときは null |
| `inbox` | object か null | `count` (帰属する msg の件数) と `last_at` (最後の msg の時刻、epoch 秒、無ければ null)。transcript が無い、または最初の時刻が取れない pane は null。run が無いときは `count` 0 |
| `queue` | object か null | `pending` (transcript の queue-operation で enqueue されて、dequeue / remove / popAll でまだ出ていない入力の件数。未到達でキュー待ち。dequeue が書かれず user 行・queued_command で届いた入力も届いた扱い)、`oldest_at` (その最古の enqueue の時刻、epoch 秒、無ければ null)、`last_input_at` (turn の起点と、キューから出て届いた入力の時刻の最大、epoch 秒、無ければ null。届いた入力は dequeue と、reason が `dropped_by_hook` 以外の remove)。transcript が無い、または最初の時刻が取れない pane は null。`working_secs` は変えない |
| `anomaly` | string か null | status が working / blocked の pane だけ、画面の末尾 20 行 (`herdr pane read <PANE_ID> --lines 20`) を読み、一覧 (src/json.rs の定数 `ANOMALY_PATTERNS`。文字列の配列で、足すときはここに足す) の文字列を含めば、その一覧の要素を返す (複数なら配列の先頭側)。画面の文字列は出さない。それ以外の status、一致なし、`pane read` の失敗は null (失敗は `warnings` に積まない)。`--once` の画面には出さない |
| `hidden` | boolean | 画面の既定で隠す pane か。常駐・dispatch で、blocked でないもの。`--show-resident` では変わらない |
| `hidden_reason` | string か null | `resident` / `dispatch`。hidden でなければ null |
| `owner` | string か null | 常駐は label が `watch-` で始まるときその label、それ以外は null。dispatch は `<repo略称>#<番号>` (label 末尾の `-bundle` は落とす)。blocked で hidden でない pane も持つ。分類されなければ null |

- dispatch の分類は `--runs-dir` の run が読めるときだけ付く。読めなければ `hidden` は false、`owner` は null
- socket のパス、transcript のパス、session_id は出さない
- `--once` との対応 (試験で確かめている)
  - `hidden` が false で status が blocked か working の pane は、`--once` の個別の行と 1 対 1
  - `hidden` が false で status が idle / unknown の pane の数の合計は、`--once` が畳んだ件数の合計と等しい
  - `--once --show-resident` の個別行と集計の件数の合計は、JSON の done 以外の pane 数と等しい

## Timeline

選んだ行の Timeline は一覧と一緒に常に出す (`--once` と、stdin が tty でないときは出さない)。選択を変えると中身も追う。端末が 120 桁以上なら一覧を左、Timeline を右に並べ (Timeline は描画幅の 35%、最小 20 桁・最大 120 桁。一覧は 70 桁を下回らず、残りを使う)、未満なら一覧を上、Timeline を下 (高さ 6 : 4) に積む。Timeline の行は積む配置と `--timeline PANE_ID` の 1 回出力でも 120 桁までで、`--width` を広く固定しても超えない。選択が無いときは「選択なし」を出す。

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

`paneListObserved` は `src/data.rs` の `gather` に対応する。配列を取得できなければ `none`、取得できれば整形済みの pane ID の列を入力とする。欠落や非文字列の ID は空文字列で表す。空の配列は観測済みで、空の ID を含む配列は未観測となる。`retainedPaneIds` は同じ入力から空の ID の行を除く処理を表す。JSON の解釈、ID の整形、socket の正規化は Rust が行う。

未処理報告の窓口状態を判定する `runWindow` は、`src/data.rs` の `read_runs` に対応する。不在を確定できるのは対象 socket の pane 一覧が観測済みの場合だけで、稼働中にするには pane ID と socket の両方の一致が必要。CLI テストは模擬 Herdr の応答を使い、空の一覧、取得失敗、不正な一覧、別セッションでの挙動を確かめる。

`tabTarget` は `src/data.rs` の `gather` で決める Enter の移動先に対応する。複数 session の一覧で別 session の行を選んだ場合と、tab ID が空の場合は移動先がない。移動先がある場合、`src/main.rs` の `focus_tab` は一覧取得時の socket 指定 (明示パスまたは Herdr の暗黙指定) と tab ID を引き継ぐ。socket の探索と正規化は Rust が行い、PTY を使う CLI テストで代替 socket と同じ tab ID を持つ別 session の挙動を確かめる。

証明は `cd herdr-timeline/lean && lake build --wfail`、実装との対応はリポジトリ直下から `cargo test --locked -p herdr-timeline` で確認する。CI では両方を実行する。
