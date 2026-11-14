# 0001: secret-scan を bash-gates の subcommand にする

## 状況

commit と投稿の gate が使う secret 検出器は、Python 3 の単体 script だった。次の問題があった。

- 1 回の起動が約 30 ms かかり、その大半が interpreter の起動と正規表現の組み立て
- IPv4 / IPv6 の `is_global` の判定を標準ライブラリに任せているので、`PATH` の `python3` の版で判定が割れる (3.9 と 3.14 で予約レンジの表が違う)
- 検出ルールと test が、利用側の private repo にあった

## 決定

`bash-gates secret-scan --policy <path> --repo <owner/name>` として、同じ binary に検出器を持つ。Release の asset として配る。

### 入出力の契約

- stdin に `git diff --unified=0` 形の unified diff を渡す。検査するのは追加行だけ
- stdout に `{"findings":[{"file","line","class","rule","value"}],"scanned":bool}` を返す。`scanned` が false のときは、policy に載っていない (または null の) repo
- exit 0 は走査完了 (検出の有無は stdout を見る)、exit 2 は引数または対象 repo の policy の不正
- credential 系の `value` は、7 文字以上なら先頭 6 文字と `…`、6 文字以下なら `…` だけを返し、検出した秘密の全体を報告に再掲しない

呼び出し側は、`scanned` が true で `findings` が配列のときだけ結果を信用する。subcommand を持たない古い binary は、exit 0 で stdout が空になる。その場合も検出なしとみなさず、呼び出し側の fail の向き (deny) に倒す。

### dispatcher より前で分岐する

`secret-scan` は `main()` の先頭で分岐し、dispatcher の stale 判定・子 gate の exec・stdin の読み込みより前に抜ける。diff を stdin に渡すので、dispatcher 側が先に読んではいけない。dispatcher の起動時間は変わらない (実測 約 3.3 ms)。

### 正規表現は fancy-regex

先読み・後読み (`(?<![A-Za-z0-9])` など) が要るので、`regex` crate ではなく `fancy-regex` を使う。Python の `re` との差は次のとおり吸収する。

- `\s`: Python は `\x1c`-`\x1f` も空白とみなす。`\s\x1c-\x1f` に展開する
- `(?i)`: Python は `İ` (U+0130) と `ı` (U+0131) を `i` に畳み込む。`i` と英字クラスに足す
- `\d`: どちらも Unicode の数字全般に当たり、集合は同じ (Python 3.14 の Unicode 16.0 の数字 760 文字で確認)。CIDR の prefix 長は Unicode の数字から値を引く
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
  - 約 10 万ケース (801 file)。全 35 ルールと、各ルールの境界値・文字種の変異・diff の形・policy の除外を含む。値はすべて実行時に連結して作った合成値
  - IPv4 / IPv6 の予約レンジの境界値は、約 41 万行 (約 20 万のアドレスを複数の表記で) で全一致
- `cargo test` に、検出ルールごとの「止める / 止めない」の table 駆動 test (`tests/secret_scan.rs`) を置く
- 1 回の実行時間 (追加 2 行の diff): Python 約 29 ms、Rust 約 5.7 ms。追加 200 行の diff では Python 約 31 ms、Rust 約 9.7 ms
