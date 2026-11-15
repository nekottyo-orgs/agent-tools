# 0001: secret-scan を bash-gates の subcommand にする

- 状態: [0002](0002-separate-binary.md) で置き換え
- 日付: 2026-11-15

> secret-scan を別 crate に分けたので、置き場を `bash-gates/docs/adr/` から `secret-scan/docs/adr/` に移した。

> 別 binary に分けた決定は [0002](0002-separate-binary.md) にある。0002 が引き継ぐ節 (入出力の契約・正規表現・`is_global`・policy) は、この ADR の本文が今も正。
> dispatcher (`bash-gates`) 側の設計は [bash-gates の ADR 0001](../../../bash-gates/docs/adr/0001-dispatcher.md) にある。

## 背景

commit と投稿の gate が使う secret 検出器は、Python 3 の単体 script だった。次の問題があった。

- 1 回の起動が約 30 ms かかり、その大半が interpreter の起動と正規表現の組み立て
- IPv4 / IPv6 の `is_global` の判定を標準ライブラリに任せているので、`PATH` の `python3` の版で判定が割れる (3.9 と 3.14 で予約レンジの表が違う)
- 検出ルールと test が、利用側の private repo にあった

## 決定

`bash-gates secret-scan --policy <path> --repo <owner/name>` として、同じ binary に検出器を持つ。Release の asset として配る。

### 入出力の契約

- stdin に `git diff --unified=0` 形の unified diff を渡す。検査するのは追加行だけ
  - `@@ -a,b +c,d @@` の行数を数え、hunk の中の `+` 始まりの行は、内容が `++ ` で始まっていても追加行として読む。hunk の外で file header とみなすのは `+++ ` (直後が空白) だけで、`+++x` や `+++` だけの行は追加行
  - hunk header は ASCII の数字だけの `@@ -a[,b] +c[,d] @@` で、数字は u64 に収まるものに限る。hunk の外でこれを読めないときは走査を失敗させる ([0002 の「読めない入力は走査の失敗にする」](0002-separate-binary.md#読めない入力は走査の失敗にする))
  - 行は git と同じく `\n` だけで分ける (CRLF の `\r` は内容に含めない)。`\x0b`、`\x0c`、`\x1c`-`\x1e`、U+0085、U+2028、U+2029 は行の内容の一部で、hunk の行数の勘定を割らず、行の途中にあっても後ろまで走査する
  - file header の path は、git が空白を含む名前の後ろに足す tab の手前までを取り、前後の空白は残す (末尾に空白のある別 file を、除外 path と同じに扱わないため)
  - git が quote した path (`"b/docs/\346..."`) は戻した実際の file 名で、除外の照合と報告に使う
  - 行に `secret-scan-gate:allow` を含む追加行は、全ルールの対象から外す (行内の除外)
- stdout に `{"findings":[{"file","line","class","rule","value"}],"scanned":bool}` を返す。`scanned` が false のときは、policy に載っていない (または null の) repo
- exit 0 は走査完了 (検出の有無は stdout を見る)、exit 2 は引数・対象 repo の policy の不正、stdin の読み取り失敗、stdin が UTF-8 でない、hunk header が読めない。exit 2 のときは stdout に何も出さない
  - 失敗の種類は、stderr に `secret-scan: error=<token>` の 1 行で出す。呼び出し側が構造化ログに残して原因を数えるため。token は `bad_arguments`、`bad_policy`、`stdin_read_failed`、`invalid_utf8`、`bad_hunk_header`、`internal_error` (panic) の 6 つで、入力の断片や path を含まない。JSON の `error` にしなかったのは、stdout を空に保つ契約を崩さないため。終了コードを見ずに stdout の `findings` を数える呼び出しが、失敗を 0 件と読む経路を作らない。token 行の次の行に、人が読む詳細が付く (policy の読み込みの失敗などは、その内容を含みうる)
- credential 系の `value` は、7 文字以上なら先頭 6 文字と `…`、6 文字以下なら `…` だけを返し、検出した秘密の全体を報告に再掲しない。credential の一致に重なる別 class の検出 (接続文字列の password をメールの形として拾う、など) は、値の一部も出さず `…` だけを返す。重なりは credential の一致全体で見るので、一致が host まで含む接続文字列 (jdbc など) では、host の IP やメールの値も `…` になる。検出の有無・位置・rule は残る

呼び出し側は、`scanned` が true で `findings` が配列のときだけ結果を信用する。subcommand を持たない古い binary は、exit 0 で stdout が空になる。その場合も検出なしとみなさず、呼び出し側の fail の向き (deny) に倒す。

### dispatcher より前で分岐する

> 別 binary に分けたので、この分岐は今は無い ([0002](0002-separate-binary.md))。

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

読めない入力の扱いは、別 binary に分けたあとに決めたので [0002 の「読めない入力は走査の失敗にする」](0002-separate-binary.md#読めない入力は走査の失敗にする) に移した。

## 検証

- 従来の Python 実装と同じ入力に当て、出力 (JSON を正規化したもの) と exit code を比べた
  - 約 10 万ケース (801 file)。分けた時点の Rust 実装では、609 file が一致し、192 file に差があった。全 35 ルールと、各ルールの境界値・文字種の変異・diff の形・policy の除外を含む。値はすべて実行時に連結して作った合成値
  - 差の 192 file は次の 5 つ。うち 173 file と、path の扱いが違う 3 file、行番号だけの差の 1 file は意図した差で、不正な UTF-8 の 4 file と hunk header の 11 file は、検出の有無が変わる入力を含んでいたので修正した ([0002 の「漏れ検証で見つかった差と修正」](0002-separate-binary.md#漏れ検証で見つかった差と修正))
    - 別 class の検出が credential の一致に重なり、値が `…` だけになる入力 (173 file)。検出の有無・位置・rule は同じで、`value` だけが違う
    - hunk header の形が壊れている、または Unicode の数字や u64 に収まらない大きさの数字がある入力 (11 file)。行番号だけの差ではない。Python は header を読めたものとして続き、Rust は 0 行の hunk として流すので、続く `+++ ` 始まりの追加行の扱いが割れ、検出の有無が変わる入力があった (漏れ検証で見つかった)。修正後は、Rust が exit 2 で止める
    - file header の path に、tab 以降の文字列、前後の空白、quote がある入力 (3 file)。Rust 側は tab の手前までを path とし、空白は残し、quote は戻した file 名にする
    - hunk の行数が実際と合わず、hunk の途中で次の hunk header が来る入力 (1 file)。Rust 側は header を読み直す。行番号だけが違う
    - 不正な UTF-8 (4 file)。当初の Rust 側は lossy に変換して走査を続けたが、検出が外れる入力があったので、修正後は exit 2 で止める
  - IPv4 / IPv6 の予約レンジの境界値は、約 41 万行 (約 20 万のアドレスを複数の表記で) で全一致
- `cargo test` に、検出ルールごとの「止める / 止めない」の table 駆動 test (`tests/secret_scan.rs`) を置く
- 1 回の実行時間 (追加 2 行の diff): Python 約 32 ms、Rust 約 8.5 ms。追加 200 行の diff では Python 約 34〜41 ms、Rust 約 10〜13 ms (Python は 3.14。100 回の平均を 2 回測った範囲)

別 binary に分けた決定と「漏れ検証で見つかった差と修正」は [0002](0002-separate-binary.md) に移した。
