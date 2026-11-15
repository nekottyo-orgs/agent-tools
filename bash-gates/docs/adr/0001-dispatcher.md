# 0001: Bash の PreToolUse gate を 1 つの dispatcher で束ねる

- 状態: 採用
- 日付: 2026-11-15

## 背景

この ADR は、dispatcher を入れた commit c1cdcb0 より後に、code と commit 履歴から起こした後追いの記録である。c1cdcb0 は PR を通らずに main へ直接入っている。この repo に入る前の経緯は記録なし。

Bash の PreToolUse hook として動く gate が複数ある。`src/main.rs` の冒頭のコメントには、次の事実がある。

- 律速は bash と jq の起動。gate を別々に登録すると、1 回に約 60 ms 待つ
- きっかけ語の無いコマンドは、実測で 52%

約 60 ms と 52% の測り方 (対象の期間、件数) は記録なし。

## 決定

Bash の PreToolUse gate 群を 1 つのプロセス `bash-gates` で受ける。command に各 gate のきっかけ語があるものだけを並列に起動し、子の結果を 1 つにまとめて返す。

### 起動する gate を選ぶ

- きっかけ語は、manifest の `dispatched` から実行時に読む。対象は、dispatcher 経由と指定された子だけ。gate を足したときに code を直し忘れる経路を作らないため。読む費用は起動の費用に比べて小さい
- 子ごとのきっかけ語は 2 種類。`words` はコマンド語として現れたときに当たる (前後が `[A-Za-z0-9_.-]` 以外か、行頭・行末)。`substrings` は部分文字列として含まれれば当たる
- 照合は、生の command と、そこから `\` `"` `'` を消した command の両方に掛ける。どちらかに当たれば起動する。gate ごとに quote の外し方が違うので、片方だけでは子の照合の上位集合にならない
- 1 本も当たらなければ、子を起動せずに stdout を空で返す

### 子の結果をまとめる

- deny が 1 つでもあれば deny、無ければ ask が 1 つでもあれば ask
- 理由文は、同じ文を 1 つにまとめる。複数残れば、子の名前を `[名前]` の見出しにして空行で連結する
- 子の `additionalContext` と `systemMessage` は、判定の有無によらず空行で連結して返す
- 判定 (deny、ask)、`additionalContext`、`systemMessage` のどれも無ければ、stdout を空で返す

### fail の向き

| 状況 | 扱い |
| --- | --- |
| 入力 JSON が壊れている | 全員を起動する |
| command が文字列でない | 全員を起動する |
| manifest が読めない | deny |
| dispatcher 経由の子が 0 本 | deny |
| 一時領域を作れない | deny |
| 子を起動できない | deny |
| dispatcher の panic | deny |
| 子が exit 2 | deny。理由は子の stderr |
| 子が 0 と 2 以外の非 0 で終わる | 素通り |
| 子が exit 0 で、出力が JSON として読めない | 素通り |
| 子の timeout | 素通り。子は SIGKILL で止める |
| 子が signal で終わる (timeout 以外) | 素通り。終了コードが取れないため |

- 親が子を起動できないとき、素通りで返すと、fail-close の gate (secret の検査など) の対象まで検査されずに通る。そのため deny に倒す
- 迷ったときに全員を起動する理由は記録なし
- 子の非 0・壊れた出力・timeout を素通りにする理由は記録なし。code のコメントには「素通りが契約」とだけある
- timeout は manifest の子ごとの値で、既定は 10 秒

### 子との受け渡し

- 子の stdin・stdout・stderr はファイルにする。孫 (jq、lint) がパイプを握ったまま残ると、子を kill しても読み取りが返らないため
- stdin には command 全文が入り、secret を含みうる。一時領域は本人だけが読める 0700 で作る

### 終了コード

終了コードは常に 0 で、判定は stdout の JSON で返す。例外は、下の stale 時の exec で、その場合は exec した先の終了コードになる。

子を起動した後に dispatcher が非 0 で抜けると、hook の登録の `||` が退避側を走らせ、子が二重に動く。panic も `catch_unwind` で受けて deny の JSON にし、0 で抜ける。

### binary が source より古いとき

`Cargo.toml`、`Cargo.lock`、`src/main.rs` のどれかが binary より新しいときは、stdin を読む前に exec する。exec 先は取り込み側が置く退避用の script で、build し直させる。

- exec に失敗したら、古い binary のまま続ける。止めるより、古い判定で通す方が害は小さいため
- `BASH_GATES_FRESH` が設定されていれば、新旧の比較を飛ばす。退避用の script が build の直後に付ける

### 環境変数

テストを隔離するために、次の 3 つを読む。

- `BASH_GATES_DIR`: 子の hook の置き場
- `BASH_GATES_MANIFEST`: manifest の path
- `BASH_GATES_FRESH`: 設定されていれば source との新旧の比較を飛ばす

### 検証

- 判定部 (`has_word`、`triggered`、`select`) と合成部 (`joined`、`merge`) を Lean のモデル (`lean/BashGates.lean`) に写し、その性質を証明する。証明の対象はモデルで、Rust との一致は `cargo test` が突き合わせ表 (`lean/vectors.txt`) を読んで関数単位で確かめる
- spawn・timeout・kill・stale 時の exec・JSON の解釈は、Lean のモデルの対象外。serde と Lean の JSON は重複キーなどで挙動が違いうるので、解釈はモデルに入れない
- 回帰テスト (`tests/bash-gates.test.sh`) は、本物の子を使わない。`BASH_GATES_DIR` に置いた偽の子と小さな manifest で、きっかけ語の照合と合成の規則を確かめる
- 回帰テストは Lean の証明が warning と sorry なしで通ることと、モデルから作り直した表が commit 済みの表と同じことも確かめる。lean が無ければ Lean の検査を、cargo が無ければ binary を使う検査を skip する
- CI は build と回帰テストを走らせる。commit 5292d80 の時点では、Rust 1.98、Lean v4.35.0、elan v4.2.4 に固定していた

### Rust で書く

律速が interpreter (bash と jq) の起動なので、dispatcher 自体はそれを起動しない binary にする。

他の言語でなく Rust を選んだ理由と、比べた言語は記録なし。

### secret-scan の subcommand を消す

v0.1.2 で足した subcommand の `bash-gates secret-scan` は、commit 89400c9 で別 binary に分けた。subcommand の削除なので、commit f7407c7 で version を minor の 0.2.0 に上げた。

分けた理由と比べた案は [secret-scan の ADR 0001 の追補](../../../secret-scan/docs/adr/0001-secret-scan-subcommand.md#追補-別-binary-に分けた) を参照。

## 検討した代替案と捨てた理由

A〜G は、`src/main.rs` のコメントにある理由から起こした案である。当時これらを実際に比べたかは記録なし。

| 案 | 内容 | 捨てた理由 |
| --- | --- | --- |
| A. gate を別々に登録 | hook ごとに Bash の PreToolUse へ登録 | 1 回に約 60 ms 待つ |
| B. きっかけ語を code に持つ | dispatcher に語の一覧を埋め込む | gate を足したときに直し忘れる |
| C. 照合を片方だけに掛ける | 生の command だけ、など | 子の照合の上位集合にならない |
| D. 起動できないときに素通り | manifest の不備などで通す | fail-close の gate の対象が検査されずに通る |
| E. 子とパイプでつなぐ | stdio をパイプにする | 孫が握ると読み取りが返らない |
| F. 失敗時に非 0 で抜ける | panic などを終了コードで返す | 退避側が走り、子が二重に動く |
| G. exec の失敗で止める | 古い binary では判定しない | 古い判定で通す方が害が小さい |
| H. secret-scan を subcommand のまま | 同じ binary に検出器を持つ | secret-scan の ADR 0001 の追補を参照 |

## 影響

### 良い面

- きっかけ語の無いコマンド (実測 52%) では、子を 1 本も起動しない
- gate を足すときは manifest に足すだけで、dispatcher の code は変えない
- 親が子を起動できない故障では deny になる。この故障で fail-close の gate の対象が検査されずに通ることは無い
- 判定部と合成部は、Lean のモデルと関数単位で一致を確かめている

### 悪い面

- 子が 0 と 2 以外の非 0 で終わる、exit 0 で出力が壊れている、timeout する、のどれでも素通りになる。その子の gate は、その回は効果を持たない
- exec が失敗すると、source より古い binary の判定で通る
- spawn・timeout・kill・stale 時の exec・JSON の解釈は、Lean のモデルでは確かめていない。cargo が無い環境では、回帰テストのうち binary を使う検査が skip される
- command 全文が一時領域のファイルに書かれる。一時領域は 0700 で作り、終わったら消そうとするが、消える保証は無い。子を待つ thread を作れずに panic すると片付けまで届かず、消すときのエラーも無視する
