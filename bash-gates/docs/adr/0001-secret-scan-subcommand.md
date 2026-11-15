# 0001: secret-scan を bash-gates の subcommand にする

> 追補 (2026-11-15): `secret-scan` は別 binary に分けた (末尾の「追補: 別 binary に分けた」)。本文の `bash-gates secret-scan <引数>` は、`secret-scan <引数>` と読み替える。入出力の契約・正規表現・`is_global`・policy・不正な UTF-8 の扱いは変わらない。「dispatcher より前で分岐する」の節は、分岐そのものが無くなったので該当しない。
> ファイル名と場所は、他の文書や取り込み側がこの path を参照しているので変えていない。

## 状況

commit と投稿の gate が使う secret 検出器は、Python 3 の単体 script だった。次の問題があった。

- 1 回の起動が約 30 ms かかり、その大半が interpreter の起動と正規表現の組み立て
- IPv4 / IPv6 の `is_global` の判定を標準ライブラリに任せているので、`PATH` の `python3` の版で判定が割れる (3.9 と 3.14 で予約レンジの表が違う)
- 検出ルールと test が、利用側の private repo にあった

## 決定

`bash-gates secret-scan --policy <path> --repo <owner/name>` として、同じ binary に検出器を持つ。Release の asset として配る。

### 入出力の契約

- stdin に `git diff --unified=0` 形の unified diff を渡す。検査するのは追加行だけ
  - `@@ -a,b +c,d @@` の行数を数え、hunk の中の `+` 始まりの行は、内容が `++ ` で始まっていても追加行として読む。hunk の外の `+++ ` だけを file header とみなす
  - 行は git と同じく `\n` だけで分ける (CRLF の `\r` は内容に含めない)。`\x0b`、`\x0c`、`\x1c`-`\x1e`、U+0085、U+2028、U+2029 は行の内容の一部で、hunk の行数の勘定を割らず、行の途中にあっても後ろまで走査する
  - file header の path は、git が空白を含む名前の後ろに足す tab の手前までを取り、前後の空白は残す (末尾に空白のある別 file を、除外 path と同じに扱わないため)
  - git が quote した path (`"b/docs/\346..."`) は戻した実際の file 名で、除外の照合と報告に使う
  - 行に `secret-scan-gate:allow` を含む追加行は、全ルールの対象から外す (行内の除外)
- stdout に `{"findings":[{"file","line","class","rule","value"}],"scanned":bool}` を返す。`scanned` が false のときは、policy に載っていない (または null の) repo
- exit 0 は走査完了 (検出の有無は stdout を見る)、exit 2 は引数・対象 repo の policy の不正、または stdin の読み取り失敗
- credential 系の `value` は、7 文字以上なら先頭 6 文字と `…`、6 文字以下なら `…` だけを返し、検出した秘密の全体を報告に再掲しない。credential の一致に重なる別 class の検出 (接続文字列の password をメールの形として拾う、など) は、値の一部も出さず `…` だけを返す。重なりは credential の一致全体で見るので、一致が host まで含む接続文字列 (jdbc など) では、host の IP やメールの値も `…` になる。検出の有無・位置・rule は残る

呼び出し側は、`scanned` が true で `findings` が配列のときだけ結果を信用する。subcommand を持たない古い binary は、exit 0 で stdout が空になる。その場合も検出なしとみなさず、呼び出し側の fail の向き (deny) に倒す。

### dispatcher より前で分岐する

`secret-scan` は `main()` の先頭で分岐し、dispatcher の stale 判定・子 gate の exec・stdin の読み込みより前に抜ける。diff を stdin に渡すので、dispatcher 側が先に読んではいけない。dispatcher の起動時間は変わらない (実測 約 3.3 ms)。

### 正規表現は fancy-regex

先読み・後読み (`(?<![A-Za-z0-9])` など) が要るので、`regex` crate ではなく `fancy-regex` を使う。Python の `re` との差は次のとおり吸収する。

- `\s`: Python は `\x1c`-`\x1f` も空白とみなす。`\s\x1c-\x1f` に展開する
- `(?i)`: Python は `İ` (U+0130) と `ı` (U+0131) を `i` に畳み込む。`i` と英字クラスに足す
- `\d`: どちらも Unicode の数字全般に当たり、集合は同じ (Python 3.14 の Unicode 16.0 の数字 760 文字で確認)。CIDR の prefix 長は Unicode の数字から値を引く
- 探索上限 (backtrack の上限) を超えた行は、見落とさないよう検出側に倒す。rule 名つきで値が `…` の検出になるので、巨大な 1 行 (minified の bundle など) は `secret-scan-gate:allow` か policy の除外で外す
- 検出の順序は、ルール表の順 → 行内の出現順。Python の `finditer` と同じ

### `is_global` の基準は Python 3.14

標準ライブラリの `is_global` は unstable で版差もあるので使わず、Python 3.14 の `ipaddress` の予約レンジの表をそのまま持つ。Python 3.15 の表は 3.14 と同一であることを確認した。3.9 とは `192.0.0.0/24`、`2002::/16`、`3fff::/20` などが食い違う。利用側の `python3` の版に判定が依存しなくなる。

### policy は呼び出し側が渡す

binary と public repo が持つのは、vendor 共通の形のルールだけ。対象 repo の一覧・repo ごとの除外 (path と class の組)・エントロピーの閾値は policy JSON に置き、利用側が `--policy` で渡す。

```json
{
  "entropy": {"generic": 3.0, "db": 2.0},
  "repos": {
    "owner/name": {"exempt": [{"paths": ["docs/**"], "classes": ["net-*", "credential/generic"]}]}
  }
}
```

- `paths` は `fnmatch` と同じ解釈 (`*` は `/` も越える)
- `classes` の `/` を含む指定は `class/rule` の完全一致、含まない指定は class への `fnmatch`
- 形式が不正な policy は exit 2

社内固有の token 形式を足したくなったときは、policy 側に利用者定義のルールを足す形にし、binary 側のルール表は vendor 共通のまま保つ (今は必要がないので実装しない)。

### 不正な UTF-8

従来の Python 実装は、UTF-8 locale で不正な UTF-8 の入力を受けると例外終了した。Rust 実装は lossy に変換して走査を続ける。ASCII 側の秘密は検出でき、他の encoding の file を 1 つ含むだけで commit が止まり続ける、という状況は起きない。

## 検証

- 従来の Python 実装と同じ入力に当て、出力 (JSON を正規化したもの) と exit code を比べた
  - 約 10 万ケース (801 file)。609 file が一致し、192 file は意図した差 (下記)。全 35 ルールと、各ルールの境界値・文字種の変異・diff の形・policy の除外を含む。値はすべて実行時に連結して作った合成値
  - 差の 192 file は次の 5 つ。不正な UTF-8 の 4 file を除き、検出した class と rule の組の件数は全 file で同じ
    - 別 class の検出が credential の一致に重なり、値が `…` だけになる入力 (173 file)。検出の有無・位置・rule は同じで、`value` だけが違う
    - hunk header の形が壊れている、または Unicode の数字や u64 に収まらない大きさの数字がある入力 (11 file)。行番号だけが違う
    - file header の path に、tab 以降の文字列、前後の空白、quote がある入力 (3 file)。Rust 側は tab の手前までを path とし、空白は残し、quote は戻した file 名にする
    - hunk の行数が実際と合わず、hunk の途中で次の hunk header が来る入力 (1 file)。Rust 側は header を読み直す。行番号だけが違う
    - 不正な UTF-8 (4 file)。Rust 側は lossy に変換して走査を続ける
  - IPv4 / IPv6 の予約レンジの境界値は、約 41 万行 (約 20 万のアドレスを複数の表記で) で全一致
- `cargo test` に、検出ルールごとの「止める / 止めない」の table 駆動 test (`tests/secret_scan.rs`) を置く
- 1 回の実行時間 (追加 2 行の diff): Python 約 32 ms、Rust 約 8.5 ms。追加 200 行の diff では Python 約 34〜41 ms、Rust 約 10〜13 ms (Python は 3.14。100 回の平均を 2 回測った範囲)

## 追補: 別 binary に分けた

`secret-scan` を `bash-gates` の subcommand から、同じ repo・同じ Cargo workspace の別 crate (`secret-scan/`) と別 binary に分けた。コードは移しただけで、検出の挙動は変えていない。

### 比べた案

| 案 | 内容 | 判断 |
| --- | --- | --- |
| A. subcommand のまま | `bash-gates secret-scan`。asset と release は 1 つ | 採らない |
| B. 別 binary・同じ repo・同じ workspace | `secret-scan` を独立した crate・tag・asset にする | 採る |
| C. 別 repo | `secret-scan` を新しい repo に出す | 今は採らない |

B を採る理由は、手元で測れた次の事実による。

- 依存が分かれる: dispatcher (`bash-gates`) は検出器のための `fancy-regex` を持たなくなる。`cargo tree` で数えた依存 crate は 21 から 15 (自分自身を含む) に減り、binary は約 1.9 MB から約 0.48 MB になった (macOS arm64、release build)。`secret-scan` は約 1.7 MB。dispatcher の 1 回の起動時間は、200 回の平均で 3.0 ms と 3.2 ms (分割前と後) で、差は測定のばらつきの範囲に収まる。したがって速くなった、とは言えない。起動時間を理由にはしていない
- release が独立する: 検出ルールの追加 (Datadog など) は頻繁に起きるが、dispatcher の挙動は変わらない。subcommand のままだと、ルールを足すたびに dispatcher の release と入れ替えが起き、利用側は Bash を打つたびに起動する binary まで取り直すことになる。別 binary なら、変えた方だけが release される。tag は tool ごとに `bash-gates/vX.Y.Z` と `secret-scan/vX.Y.Z`、asset は `<tool>-<target>` と `<tool>-<target>.sha256`
- 取り違えの経路が消える: subcommand を持たない古い `bash-gates` は、知らない引数を無視して exit 0 と空の出力を返す。呼び出し側が終了コードだけを見ると「検出 0 件」に化ける。`secret-scan` は subcommand が無く、引数なしと未知の引数 (`secret-scan secret-scan ...` のような旧い呼び方を含む) は exit 2 で、stdout に何も出さない。`tests/secret_scan.rs` の `bad_arguments_exit_2` で固定している
- 証明の対象が混ざらない: `bash-gates/lean/` のモデルは `bash-gates` の関数だけを対象にする。検出器の変更で、その検査の入力が変わらない

A を採らない理由は上の 3 点。C を今は採らない理由は、`bash-gates` と `secret-scan` が共有するコードが無く、独立した release と依存の分離は B で得られること。repo を分けると、CI・release の自動化・Release の設定 (immutable releases) と GitHub App の権限を repo ごとに持つ手間が増える。共有コードが生まれる、または権限と公開範囲を分けたくなったら再検討する。

### 呼び出し側の扱い

- 通過の条件は変えない。exit 0 で、stdout の `scanned` が true、`findings` が配列のときだけ結果を信用する。別 binary でも、path に別の実行ファイル (古い `bash-gates` など) が置かれていれば exit 0 と空の出力を返しうる。fail-close の gate がそれを「検出 0 件」と読むと、検査が黙って外れる。確認の費用は JSON の 1 回の検査で、残す
- `secret-scan` は自分の古さを判定しない。`bash-gates` のように、source との新旧で入れ替えを促す経路を持たない。pin を上げても binary は自動では入れ替わらないので、取り込み側が binary の sha256 を pin と比べて取り直す

### 帰結

- `bash-gates` から `secret-scan` の subcommand を消した。`bash-gates secret-scan` は dispatcher として hook の JSON を読もうとするだけで、検出はしない。取り込み側は呼び出しを `secret-scan` binary に替えてから、`bash-gates` の pin を上げる
- root の `Cargo.toml` が workspace になり、`Cargo.lock` と build の出力 (`target/`) は repo の root に移った。`[profile.release]` は workspace の root でしか効かないので、root に置いた
- release の自動化 (tagpr) の workflow も tool ごとに分けた。tagpr は前回の tag 以降の PR を数え、path では絞らないので、push の trigger を tool のディレクトリに絞らないと、別の tool の merge で release PR が更新される
