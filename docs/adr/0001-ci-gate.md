# 0001: PR の CI を ci-gate で集約し、判定を test 可能な script にする

- 状態: 採用
- 日付: 2026-11-27

## 背景

PR #93 で、全 PR に必ず作られる集約 check `gate` を追加した。tool ごとの reusable workflow は変更 path に応じて `ci-gate` から呼び、対象外の tool は skip する。branch protection では `gate` だけを必須にする。

レビューで次の 2 点が問題になった。

- `paths` job が `git diff --name-only` の出力を行単位で `case` に掛けていた。git は既定で非 ASCII の path を引用出力 (`core.quotepath`) するため、`inbox-core/日本語.txt` のような変更がどの tool にも一致せず、suite が skip されたまま `gate` が成功する (fail-open)
- `paths` の選択と `gate` の集約が YAML 内の shell に閉じており、回帰テストが無かった。`gate` は「failure が 1 つでもあれば失敗、success と skipped だけなら成功」という重要な不変条件を持つ

## 決定

- 変更 path の選択を `.github/scripts/ci-gate/detect-tools.sh` に切り出す。入力は `git -c core.quotepath=false diff --name-only --no-renames -z` の NUL 区切り出力とし、path を引用や行分割を経ずにそのまま `case` に掛ける。出力は `GITHUB_OUTPUT` に追記できる `KEY=value` 行
- `gate` の集約を `.github/scripts/ci-gate/check-results.sh` に切り出す。引数は `job名=result` とし、`success` と `skipped` だけを成功、それ以外 (`failure` / `cancelled` / 未知の result) を失敗として扱う。結果の渡し忘れ・形式不正も、設定の取りこぼしを成功にしないため失敗にする。さらに必須 job の集合 (`paths` / `bash-gates` / `herdr-timeline` / `inbox-core` / `secret-scan`) を script 内に固定し、引数の job 名がその集合とちょうど一致することを要求する。欠落・重複・未知の job 名はいずれも失敗にする (呼び出し側の取りこぼしや typo による fail-open を防ぐ)
- 回帰テストを `.github/scripts/ci-gate/tests/ci-gate.test.sh` に置き、`paths` job で毎回実行する。component workflow の jobs は増やさない
- [AGENTS.md](../../AGENTS.md) の形式検証として、`gate` の集約ルールの小さな Alloy モデルを `.github/scripts/ci-gate/ci-gate.als` に置く。有限で状態遷移も並行性も無い判定だが、実装テストとは別に「必須 job がちょうど揃い、すべて success か skipped のとき、かつそのときに限り成功する」という仕様を、実装から独立した形で書くため (理由は代替案 D)
- モデルの前提・範囲と `check-results.sh` との対応は次のとおり
  - 前提: 必須 job は `check-results.sh` の `required_jobs` と同じ 5 つ。result は `success` / `skipped` / `failure` / `cancelled`。引数は `job名=result` の列で、順序は成否に影響しない
  - `Name` は必須 5 job と、それ以外の job 名 (`UnknownName`) を表す。`Result` は 4 種類と、それ以外の result (`UnknownResult`) を表す。`Entry` が 1 引数、`Input.args` が引数の列 (重複を区別するため `seq`)
  - `scriptGate` は `check-results.sh` の判定を写す。引数が 1 つ以上あり、未知の job 名が無く、`success` / `skipped` 以外の result が無く、job 名の重複が無く、必須 job がすべて現れるときだけ成功する
  - `expected` は仕様。必須 job の集合と引数の job 名の集合が一致し、引数の個数が必須 job の数 (5) に等しく、`success` / `skipped` 以外の result が無い、という形で「ちょうど揃い、すべて success か skipped」を表す
  - `Correspondence` が `scriptGate ↔ expected` を主張し、`check` で scope 内の反例を探す
  - `=` の無い引数・空の job 名・空の result は、`check-results.sh` ではいずれも失敗になる。モデルはこれらを `UnknownName` / `UnknownResult` の `Entry` にまとめ、gate の成否が同じ失敗になることだけを扱う。形式ごとの扱いは実装テストが確かめる
  - scope は 8。必須 5 job に重複や未知の job を足した反例 (最大 7 引数) まで含む。Alloy の `check` は scope 内の反例探索であり、有限の全列挙ではない
- 4^5 の実装テストは残し、モデルが表す真理表と実装の対応確認 (correspondence check) にする。真理表の全組み合わせは実装テスト、引数集合の欠落・重複・未知を含む仕様との同値はモデルが担う
  - `gate` は 5 job × 4 result の真理表。`success` / `skipped` / `failure` / `cancelled` の全組み合わせ (4^5) を列挙し、`success` と `skipped` だけなら終了コード 0、それ以外が 1 つでもあれば非 0 になることを確かめる
  - `gate` の入力集合として、各必須 job が欠けた場合、同じ job が重複した場合、未知の job 名 (既知 job 名の typo を含む) が混ざった場合も非 0 になることを確かめる
  - `paths` は代表ケースとして、各 tool 配下、`Cargo.toml` / `Cargo.lock` / `actions.lock`、各 component workflow、対象外のみ、複数 tool、非 ASCII・空白・改行を含む path を確かめる
- component workflow の `jobs` と `on.push` は変更しない

## 検証

- `bash .github/scripts/ci-gate/tests/ci-gate.test.sh`
- `actionlint -no-color .github/workflows/*.yml`
- `gh actions-lock --verify-local` (action の追加は無いが、lockfile の coverage を確認する)
- モデル: `java -jar <alloy.jar> exec -f -c Correspondence -t text -o /tmp/ci-gate-alloy .github/scripts/ci-gate/ci-gate.als`
  - Alloy 6.2.0 で実行し、`check Correspondence` が `UNSAT` (scope 8 に反例が無い) になることを確認した。`run showPass` が `SAT` (成功する入力が存在する) になることも確認した
  - 検出力の確認として、モデルの `scriptGate` が `failure` を成功として扱うように変えると `SAT` (反例あり) になり、必須 job の検査または重複の検査を外すと `SAT` になることを手元で確かめた
  - CI では回さない。Alloy の jar と Java を CI に固定する仕組みはこの PR では作らず、モデルは手元の検証に留める
- CI では `paths` job がテストを実行し、その後に実際の diff で `detect-tools.sh` を動かす。`paths` job 自体が失敗した場合も `gate` は `paths=failure` を見て失敗する

## 検討した代替案と捨てた理由

| 案 | 内容 | 捨てた理由 |
| --- | --- | --- |
| A. YAML 内の shell のまま | 判定を workflow に残す | 回帰テストを実行できず、非 ASCII の取りこぼしのような fail-open を CI で検出できない |
| B. `core.quotepath=false` だけを付ける | 行区切りのまま引用を止める | 改行を含む path では行が分かれ、別の path として判定される。NUL 区切りの方が path の表現に依存しない |
| C. 未知の result を成功として扱う | 従来どおり `failure` / `cancelled` だけを失敗にする | result の追加や取り違えが fail-open になる。許可するのは `success` と `skipped` に限定する |
| D. 実装テストだけで済ませる | 4^5 の真理表と入力集合の異常系だけを実装テストで検査し、モデルは作らない | 実装テストの期待値は、判定とは別に書いたもう 1 つの実装で、仕様そのものではない。有限で状態遷移も並行性も無くても、集約ルールが fail-open しないという仕様を実装から独立に書き、`check` で入力の組合せを反例探索する価値がある。4^5 のテストは「必須 5 job がちょうど 1 回ずつ」の入力しか列挙せず、欠落・重複・未知の job と result の組合せは代表ケースだけになる。Alloy は scope 8 でそれらを同時に動かせる。Lean は既存の tool にあるが、この判定には状態も時相も無く、帰納法より有限の関係検査の方が小さい |

## 影響

### 良い面

- 非 ASCII や改行を含む path でも tool の変更を取りこぼさない
- `paths` の選択と `gate` の集約を、YAML を読まずにローカルで検証できる
- `gate` の不変条件が真理表の全組み合わせで検査される
- 集約ルールの仕様が Alloy のモデルとして実装から独立に残り、欠落・重複・未知の job を含む入力で実装との同値を反例探索できる

### 悪い面

- `gate` job でも checkout が要るため、集約の実行に repository の取得が 1 回増える
- `paths` job が script のテストも兼ねるため、テストが落ちると suite の選択より先に job が失敗する (gate は失敗するので fail-close)
- Alloy のモデルは CI で実行しないため、モデルを変えたときは手元で `check` を回す必要がある
