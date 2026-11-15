# 0002: secret-scan を bash-gates から別 binary に分ける

- 状態: 採用
- 日付: 2026-11-15

## 背景

[0001](0001-secret-scan-subcommand.md) では、secret 検出器を `bash-gates secret-scan` という subcommand として、dispatcher と同じ binary に持たせた。分けた理由は下の「決定」にある。

### 0001 から引き継ぐもの

次のものは変えない。本文は 0001 にある。

- [入出力の契約](0001-secret-scan-subcommand.md#入出力の契約)。`bash-gates secret-scan <引数>` は `secret-scan <引数>` と読み替える
- [正規表現は fancy-regex](0001-secret-scan-subcommand.md#正規表現は-fancy-regex)
- [`is_global` の基準は Python 3.14](0001-secret-scan-subcommand.md#is_global-の基準は-python-314)
- [policy は呼び出し側が渡す](0001-secret-scan-subcommand.md#policy-は呼び出し側が渡す)

### 0001 から変えたもの

- 置き場: `bash-gates` の subcommand から、別 crate (`secret-scan/`) と別 binary に分けた
- 0001 の「dispatcher より前で分岐する」は、分岐そのものが無くなったので該当しない
- 分けたあとに、読めない入力の扱いと diff の読み方を変えた (下の「読めない入力は走査の失敗にする」と「漏れ検証で見つかった差と修正」)

## 決定

`secret-scan` を `bash-gates` の subcommand から、同じ repo・同じ Cargo workspace の別 crate (`secret-scan/`) と別 binary に分ける。分けた時点のコードは移しただけで、検出の挙動は変えていない。そのあとの修正は「漏れ検証で見つかった差と修正」に書いた。

B (下の代替案の表) を採る理由は、手元で測れた次の事実による。

- 依存が分かれる: dispatcher (`bash-gates`) は検出器のための `fancy-regex` を持たなくなる。`cargo tree` で数えた依存 crate は 21 から 15 (自分自身を含む) に減り、binary は約 1.9 MB から約 0.48 MB になった (macOS arm64、release build)。`secret-scan` は約 1.7 MB。dispatcher の 1 回の起動時間は、200 回の平均で 3.0 ms と 3.2 ms (分割前と後) で、差は測定のばらつきの範囲に収まる。したがって速くなった、とは言えない。起動時間を理由にはしていない
- release が独立する: 検出ルールの追加 (Datadog など) は頻繁に起きるが、dispatcher の挙動は変わらない。subcommand のままだと、ルールを足すたびに dispatcher の release と入れ替えが起き、利用側は Bash を打つたびに起動する binary まで取り直すことになる。別 binary なら、変えた方だけが release される。tag は tool ごとに `bash-gates/vX.Y.Z` と `secret-scan/vX.Y.Z`、asset は `<tool>-<target>` と `<tool>-<target>.sha256`
- 取り違えの経路が消える: subcommand を持たない古い `bash-gates` は、知らない引数を無視して exit 0 と空の出力を返す。呼び出し側が終了コードだけを見ると「検出 0 件」に化ける。`secret-scan` は subcommand が無く、引数なしと未知の引数 (`secret-scan secret-scan ...` のような旧い呼び方を含む) は exit 2 で、stdout に何も出さない。`tests/secret_scan.rs` の `bad_arguments_exit_2` で固定している
- 証明の対象が混ざらない: `bash-gates/lean/` のモデルは `bash-gates` の関数だけを対象にする。検出器の変更で、その検査の入力が変わらない

### 呼び出し側の扱い

- 通過の条件は変えない。exit 0 で、stdout の `scanned` が true、`findings` が配列のときだけ結果を信用する。別 binary でも、path に別の実行ファイル (古い `bash-gates` など) が置かれていれば exit 0 と空の出力を返しうる。fail-close の gate がそれを「検出 0 件」と読むと、検査が黙って外れる。確認の費用は JSON の 1 回の検査で、残す
- `secret-scan` は自分の古さを判定しない。`bash-gates` のように、source との新旧で入れ替えを促す経路を持たない。pin を上げても binary は自動では入れ替わらないので、取り込み側が binary の sha256 を pin と比べて取り直す

### 読めない入力は走査の失敗にする

次の入力は、読めたことにして走査を続けず、exit 2 で止める。呼び出し側は exit 2 を fail-close (deny) として扱う。

- 不正な UTF-8: 当初は U+FFFD に置き換えて走査していた。置き換えると、値の長さと文字の散らばりが変わる。DB の接続文字列の password の位置に不正な byte があると、エントロピーと長さが閾値を割って検出が外れた。従来の Python 実装も UTF-8 の locale では例外終了で、呼び出し側は deny していたので、deny という結果は変わらない。stdin 全体を見るので、不正な byte が削除行 (`-`) にだけある diff も止まる。代償として、UTF-8 でない text file を追加する commit に加え、そのような file を直す・消す commit も止まる (行内の pragma では外せない)。追加行だけに絞って止まる範囲を狭めるかどうかは、決めていない。許可が要る commit は、呼び出し側の bypass の手順に従う
- 読めない hunk header: 壊れた形、全角など ASCII でない数字、u64 を超える count。0 行の hunk として流すと、続く `+++ ` で始まる追加行を file header と取り違え、その行は検査されない。従来の Python 実装は全角の数字も int に直し、桁数に上限が無いので、これらの header を hunk として読む。git はどちらも出さないので、同じ読み方を再現せず、読めないと認めて止める
  - header を読むのは hunk の外にいるときと、hunk の途中で行数が合わず次の header が来たとき。hunk の中の行は内容として読むので、追加行の内容が `@@` で始まっても止まらない

hunk の外の `+++` で始まる行は、直後が空白なら file header、空白でなければ追加行として読む。呼び出し側が本文を diff に包む形では、本文の 2 行目以降がこの位置に来るので、header と取り違えると本文が検査されない。

file header の `/dev/null` かどうかは、quote や tab を戻す前の文字列で決める。git は `/dev/null` をそのまま出すので、`"/dev/null"` や `/dev/null<tab>x` は別名の file として走査する。従来の実装と同じ判定で、git が出さない形に対して検出を減らさない。

### 漏れ検証で見つかった差と修正

従来の実装は検出し、当初の Rust 実装は見逃す入力が、別の検証で見つかった。修正後の挙動を `tests/secret_scan.rs` の `invalid_utf8_exits_2`、`unreadable_hunk_header_exits_2` と diff の読み方の table に置いた。値は実行時に組み立てた合成値で、fixture に本物らしい値は置いていない。

| 入力 | 当初の Rust | 修正後 |
| --- | --- | --- |
| 不正な UTF-8 (DB の接続文字列の password の位置など) | U+FFFD に置き換えて走査。閾値を割って検出が外れる | exit 2 |
| u64 を超える count、全角の数字、壊れた hunk header | 0 行の hunk として流し、続く `+++ ` 始まりの追加行を file header と読む | exit 2 |
| hunk の外の `+++x`、`+++` だけの行 | 捨てる | 追加行として走査 (従来の実装と同じ) |
| `+++ "/dev/null"`、`+++ /dev/null<tab>x` | 追跡しない | 別名の file として走査 (従来の実装と同じ。path の表記は quote と tab を戻した形になり、従来の実装とは違う) |

## 検討した代替案と捨てた理由

| 案 | 内容 | 捨てた理由 |
| --- | --- | --- |
| A. subcommand のまま | `bash-gates secret-scan`。asset と release は 1 つ | 上の決定の理由のうち、依存・release・取り違えの 3 点 |
| B. 別 binary・同じ repo・同じ workspace | `secret-scan` を独立した crate・tag・asset にする | 捨てていない。この案を採った |
| C. 別 repo | `secret-scan` を新しい repo に出す | 今は採らない。`bash-gates` と `secret-scan` が共有するコードが無く、独立した release と依存の分離は B で得られる。repo を分けると、CI・release の自動化・Release の設定 (immutable releases) と GitHub App の権限を repo ごとに持つ手間が増える。共有コードが生まれる、または権限と公開範囲を分けたくなったら再検討する |

## 影響

### 良い面

- 検出ルールを変えても、dispatcher (`bash-gates`) の release と入れ替えは起きない
- 古い binary を呼んだときに「検出 0 件」に化ける経路が、`secret-scan` 側では exit 2 になる

### 悪い面

- `bash-gates` から `secret-scan` の subcommand を消した。`bash-gates secret-scan` は dispatcher として hook の JSON を読もうとするだけで、検出はしない。取り込み側は呼び出しを `secret-scan` binary に替えてから、`bash-gates` の pin を上げる
- `secret-scan` の binary は自動では入れ替わらない。取り込み側が sha256 を pin と比べて取り直す
- 不正な UTF-8 を含む diff は、追加行に無くても止まる (上の「読めない入力は走査の失敗にする」)

### その他の帰結

- root の `Cargo.toml` が workspace になり、`Cargo.lock` と build の出力 (`target/`) は repo の root に移った。`[profile.release]` は workspace の root でしか効かないので、root に置いた
- release の自動化 (tagpr) の workflow も tool ごとに分けた。tagpr は前回の tag 以降の PR を数え、path では絞らないので、push の trigger を tool のディレクトリに絞らないと、別の tool の merge で release PR が更新される

## 検証

分けたあとの修正 (読めない入力と diff の読み方) を含む、従来の Python 実装との突き合わせは [0001 の検証](0001-secret-scan-subcommand.md#検証) にある。
