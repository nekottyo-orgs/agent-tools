# 0001: CI を ci-gate で集約し、判定を test 可能な script にする

- 状態: 採用
- 日付: 2026-11-27

## 背景

PR #93 で、全 PR に必ず作られる集約 check `gate` を追加した。tool ごとの reusable workflow は変更 path に応じて `ci-gate` から呼び、対象外の tool は skip する。branch protection では `gate` だけを必須にする。

この形では PR で変更のあった tool の suite だけが実行され、残りは skip を成功扱いにする。そのため「main に push されるたびに、その時点の main (tip) で全 suite が通っている」という保証が無い、という指摘があった。agents repo と同じく、PR は選択実行、main はフル実行にする。

レビューで次の 2 点が問題になった。

- `paths` job が `git diff --name-only` の出力を行単位で `case` に掛けていた。git は既定で非 ASCII の path を引用出力 (`core.quotepath`) するため、`inbox-core/日本語.txt` のような変更がどの tool にも一致せず、suite が skip されたまま `gate` が成功する (fail-open)
- `paths` の選択と `gate` の集約が YAML 内の shell に閉じており、回帰テストが無かった。`gate` は「failure が 1 つでもあれば失敗、success と skipped だけなら成功」という重要な不変条件を持つ

## 決定

- 変更 path の選択を `.github/scripts/ci-gate/detect-tools.sh` に切り出す。入力は `git -c core.quotepath=false diff --name-only --no-renames -z` の NUL 区切り出力とし、path を引用や行分割を経ずにそのまま `case` に掛ける。出力は `GITHUB_OUTPUT` に追記できる `KEY=value` 行
- `gate` の集約を `.github/scripts/ci-gate/check-results.sh` に切り出す。引数は `job名=result` とし、`success` と `skipped` だけを成功、それ以外 (`failure` / `cancelled` / 未知の result) を失敗として扱う。結果の渡し忘れ・形式不正も、設定の取りこぼしを成功にしないため失敗にする。さらに必須 job の集合 (`paths` / `bash-gates` / `herdr-timeline` / `inbox-core` / `secret-scan`) を script 内に固定し、引数の job 名がその集合とちょうど一致することを要求する。欠落・重複・未知の job 名はいずれも失敗にする (呼び出し側の取りこぼしや typo による fail-open を防ぐ)
- 回帰テストを `.github/scripts/ci-gate/tests/ci-gate.test.sh` に置き、`paths` job で毎回実行する。component workflow の jobs は増やさない
- `ci-gate` に `push: branches: [main]` を追加し、PR は変更のあった tool の suite だけを実行する scoped、main への push は全 tool の suite を実行する full にする。push trigger は push イベント単位で起動し、検証するのはその push の tip commit である。複数 commit をまとめた push の途中 commit は個別には検証されず、保証は「main に push されるたびに、その時点の main (tip) に対して全 suite を実行する」に限る。この repo のマージは squash merge で 1 マージ = 1 commit = 1 push なので、main の各状態は検証済みになる。main では path 判定をせず全 tool を対象にするため、`detect-tools.sh --all` に push 用の経路を持たせる。YAML 側に全 tool の真偽値をベタ書きせず、path 判定と同じ script とテストで扱う
- component workflow 4 本から `push` trigger を外す。`workflow_call` と `jobs` は変えない。main でも入口を `ci-gate` に一本化し、suite が二重に走らないようにする
- [AGENTS.md](../../AGENTS.md) の形式検証として、`gate` の集約ルールの小さな Alloy モデルを `.github/scripts/ci-gate/ci-gate.als` に置く。有限で状態遷移も並行性も無い判定だが、実装テストとは別に「必須 job がちょうど揃い、すべて success か skipped のとき、かつそのときに限り成功する」という仕様を、実装から独立した形で書くため (理由は代替案 D)
- モデル検査は `.github/scripts/ci-gate/check-model.sh` を `paths` job から実行し、CI でも回す。Alloy は 6.2.0 に固定し、公式 release の jar を固定 URL (`https://github.com/AlloyTools/org.alloytools.alloy/releases/download/v6.2.0/org.alloytools.alloy.dist.jar`) から取得して、repo に固定した sha256 (`6b8c1cb5bc93bedfc7c61435c4e1ab6e688a242dc702a394628d9a9801edb78d`) と一致することを確認してから使う。Java は Temurin 21 を version 固定の `actions/setup-java` で用意し、その action も `actions.lock` に記録する。`check Correspondence` と `check SixPositionsRejected` は `UNSAT`、`run showPass` と `run showSixPositions` は `SAT` を要求し、反例の有無・instance の有無が想定と違う場合、または出力から結果を判定できない場合は失敗にする (fail-close)。gate の必須 job 集合 (`paths` / `bash-gates` / `herdr-timeline` / `inbox-core` / `secret-scan`) とモデルの Required は変えない
- モデルの前提・範囲と `check-results.sh` との対応は次のとおり
  - モデルの対象は `gate` の集約ルールだけ。PR と push の区別、および `paths` の選択 (`detect-tools.sh` の path 判定と `--all`) はモデル対象外とし、実装テストで扱う
  - 前提: 必須 job は `check-results.sh` の `required_jobs` と同じ 5 つ。result は `success` / `skipped` / `failure` / `cancelled`。引数は `job名=result` の列で、順序は成否に影響しない
  - `Name` は必須 5 job と、それ以外の job 名 (`UnknownName`) を表す。`Result` は 4 種類と、それ以外の result (`UnknownResult`) を表す。`Entry` が 1 引数、`Input.args` が引数の列 (重複を区別するため `seq`)
  - `scriptGate` は `check-results.sh` の判定を写す。引数が 1 つ以上あり、未知の job 名が無く、`success` / `skipped` 以外の result が無く、引数の各位置の job 名が互いに異なり (引数の集合 `Input.args.elems` ではなく、位置ごとに数える)、必須 job がすべて現れるときだけ成功する
  - `expected` は仕様。必須 job の集合と引数の job 名の集合が一致し、引数の個数 (`#Input.args`。重複した位置も数える) が必須 job の数 (5) に等しく、`success` / `skipped` 以外の result が無い、という形で「ちょうど揃い、すべて success か skipped」を表す。`#Input.args.elems` で数えると、同じ job 名の位置が集合で畳まれて 6 引数の重複を見逃す
  - `Correspondence` が `scriptGate ↔ expected` を主張し、`check` で scope 内の反例を探す
  - `SixPositionsRejected` が、必須 5 job がすべて `success` / `skipped` でも 6 position の入力 (6 個目が必須 job の重複か未知の job) は `scriptGate` も `expected` も偽になることを主張する。`showSixPositions` でその入力が存在することも確かめる。`elems` だけで重複を判定していた旧モデルではこの `check` が `SAT` (反例あり) になる
  - `=` の無い引数・空の job 名・空の result は、`check-results.sh` ではいずれも失敗になる。モデルはこれらを `UnknownName` / `UnknownResult` の `Entry` にまとめ、gate の成否が同じ失敗になることだけを扱う。形式ごとの扱いは実装テストが確かめる
  - scope は 8。必須 5 job に重複や未知の job を足した反例 (最大 7 引数) まで含む。Alloy の `check` は scope 内の反例探索であり、有限の全列挙ではない
- 4^5 の実装テストは残し、モデルが表す真理表と実装の対応確認 (correspondence check) にする。真理表の全組み合わせは実装テスト、引数集合の欠落・重複・未知を含む仕様との同値はモデルが担う。6 position の例はモデルの `SixPositionsRejected` と実装テストの両方で確かめる
  - `gate` は 5 job × 4 result の真理表。`success` / `skipped` / `failure` / `cancelled` の全組み合わせ (4^5) を列挙し、`success` と `skipped` だけなら終了コード 0、それ以外が 1 つでもあれば非 0 になることを確かめる
  - `gate` の入力集合として、各必須 job が欠けた場合、同じ job が重複した場合、未知の job 名 (既知 job 名の typo を含む) が混ざった場合も非 0 になることを確かめる
  - `paths` は代表ケースとして、各 tool 配下、`Cargo.toml` / `Cargo.lock` / `actions.lock`、各 component workflow、対象外のみ、複数 tool、非 ASCII・空白・改行を含む path を確かめる
- component workflow は `jobs` を変えず、`on.push` だけを外す。PR の scoped 実行と main の full 実行の分岐は `ci-gate` と `detect-tools.sh` が持つ

## 検証

- `bash .github/scripts/ci-gate/tests/ci-gate.test.sh`
  - PR の `paths` 選択に加え、main の `--all` (全 tool が true、対象外の path でも変わらない、未知の引数は失敗) と、`gate` の真理表・異常系を含む全ケースが pass すること
- `actionlint -no-color .github/workflows/*.yml`
  - `ci-gate.yml` の `push: branches: [main]` と、component workflow から `push` trigger を外した形を含めて検査する
- `gh actions-lock --verify-local` (`actions/setup-java` を追加したため、lockfile の coverage を確認する)
- モデル: `bash .github/scripts/ci-gate/check-model.sh`
  - script は Alloy 6.2.0 の配布 jar を固定 URL (`https://github.com/AlloyTools/org.alloytools.alloy/releases/download/v6.2.0/org.alloytools.alloy.dist.jar`) から取得し、sha256 `6b8c1cb5bc93bedfc7c61435c4e1ab6e688a242dc702a394628d9a9801edb78d` を検証してから使う。取得や検証に失敗した場合は失敗する。手元では `ALLOY_CACHE_DIR` に同じ jar を置けば再利用する
  - script が実行するコマンドは `java -jar <alloy.jar> exec -f -c <command> -t text -o <dir> .github/scripts/ci-gate/ci-gate.als` で、`Correspondence` / `SixPositionsRejected` / `showPass` / `showSixPositions` の 4 つを順に実行する
  - `check Correspondence` と `check SixPositionsRejected` が `UNSAT` (scope 8 に反例が無い)、`run showPass` と `run showSixPositions` が `SAT` (成功する入力と 6 position の入力が存在する) になることを要求する。結果が想定と違う場合や、出力から対象コマンドの結果を 1 行に特定できない場合 (未知の形式を含む) は失敗する (fail-close)
  - 検出力の確認として、モデルの `scriptGate` が `failure` を成功として扱うように変えると `check Correspondence` が `SAT` (反例あり) になり、script が失敗することを手元で確かめた。必須 job の検査または重複の検査を外すと `SAT` になる。重複の判定を `Input.args.elems` に戻すと `check SixPositionsRejected` が `SAT` になり、6 position の例が旧モデルの見逃しを検出することも確かめた
  - CI では `paths` job が上の script を毎回実行する。Alloy の jar は script が取得時に sha256 を検証し、Java は workflow が Temurin 21 を固定して用意する。`paths` job が失敗すれば `gate` は `paths=failure` で失敗する
  - 今回の変更は `gate` の集約ルールとモデルを変えないため、Alloy モデル (`ci-gate.als`) は変更しない
- CI では `paths` job が script のテストとモデル検査を実行し、その後に PR なら実際の diff、main への push なら `--all` で `detect-tools.sh` を動かす。push で各 suite が検証するのは、checkout したその push の tip commit である。`paths` job 自体が失敗した場合も `gate` は `paths=failure` を見て失敗する

## 検討した代替案と捨てた理由

| 案 | 内容 | 捨てた理由 |
| --- | --- | --- |
| A. YAML 内の shell のまま | 判定を workflow に残す | 回帰テストを実行できず、非 ASCII の取りこぼしのような fail-open を CI で検出できない |
| B. `core.quotepath=false` だけを付ける | 行区切りのまま引用を止める | 改行を含む path では行が分かれ、別の path として判定される。NUL 区切りの方が path の表現に依存しない |
| C. 未知の result を成功として扱う | 従来どおり `failure` / `cancelled` だけを失敗にする | result の追加や取り違えが fail-open になる。許可するのは `success` と `skipped` に限定する |
| D. 実装テストだけで済ませる | 4^5 の真理表と入力集合の異常系だけを実装テストで検査し、モデルは作らない | 実装テストの期待値は、判定とは別に書いたもう 1 つの実装で、仕様そのものではない。有限で状態遷移も並行性も無くても、集約ルールが fail-open しないという仕様を実装から独立に書き、`check` で入力の組合せを反例探索する価値がある。4^5 のテストは「必須 5 job がちょうど 1 回ずつ」の入力しか列挙せず、欠落・重複・未知の job と result の組合せは代表ケースだけになる。Alloy は scope 8 でそれらを同時に動かせる。Lean は既存の tool にあるが、この判定には状態も時相も無く、帰納法より有限の関係検査の方が小さい |
| E. push 用の真偽値を YAML にベタ書き | `ci-gate.yml` で push のとき 4 つの output を直接 `true` にする | path 判定の実装と、「push では全 tool」という規則が YAML と script に分かれ、片方だけ変える取りこぼしが起きる。`detect-tools.sh --all` にすれば同じ script と同じテストで確かめられる |

## 影響

### 良い面

- 非 ASCII や改行を含む path でも tool の変更を取りこぼさない
- `paths` の選択と `gate` の集約を、YAML を読まずにローカルで検証できる
- `gate` の不変条件が真理表の全組み合わせで検査される
- 集約ルールの仕様が Alloy のモデルとして実装から独立に残り、欠落・重複・未知の job を含む入力で実装との同値を反例探索できる
- モデルを変えたときに CI が `check` と `run` の結果を検証するため、手元で回し忘れても反例の見逃しや instance の消失を検出できる
- PR は変更のあった tool に絞ったまま、main への push のたびにその時点の main (tip) で全 suite が通ったことを `gate` で保証できる
- main への push でも component workflow の suite は `ci-gate` から 1 回だけ呼ばれ、二重に走らない

### 悪い面

- `gate` job でも checkout が要るため、集約の実行に repository の取得が 1 回増える
- `paths` job が script のテストも兼ねるため、テストが落ちると suite の選択より先に job が失敗する (gate は失敗するので fail-close)
- main への push は変更 path によらず全 suite を実行するため、PR の scoped 実行に比べて CI の実行時間と消費は増える (main の保証と引き換えの意図したコスト)
- Alloy の jar (約 21 MB) の取得と Java の用意を `paths` job に加えたため、PR ごとに数秒から数十秒の追加コストがかかる。jar は CI では実行ごとに取得し、手元では `ALLOY_CACHE_DIR` に同じハッシュの jar があれば再利用する
