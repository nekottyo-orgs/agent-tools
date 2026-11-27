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
- `gate` の集約を `.github/scripts/ci-gate/check-results.sh` に切り出す。引数は `job名=result` とし、`success` と `skipped` だけを成功、それ以外 (`failure` / `cancelled` / 未知の result) を失敗として扱う。結果の渡し忘れ・形式不正も、設定の取りこぼしを成功にしないため失敗にする
- 回帰テストを `.github/scripts/ci-gate/tests/ci-gate.test.sh` に置き、`paths` job で毎回実行する。component workflow の jobs は増やさない
- [AGENTS.md](../../AGENTS.md) の形式検証に対しては、状態遷移や並行実行を持たない有限な判定なので、Lean / Alloy のモデルではなく実装テストで対応する
  - `gate` は 5 job × 4 result の真理表。`success` / `skipped` / `failure` / `cancelled` の全組み合わせ (4^5) を列挙し、`success` と `skipped` だけなら終了コード 0、それ以外が 1 つでもあれば非 0 になることを確かめる
  - `paths` は代表ケースとして、各 tool 配下、`Cargo.toml` / `Cargo.lock` / `actions.lock`、各 component workflow、対象外のみ、複数 tool、非 ASCII・空白・改行を含む path を確かめる
- component workflow の `jobs` と `on.push` は変更しない

## 検証

- `bash .github/scripts/ci-gate/tests/ci-gate.test.sh`
- `actionlint -no-color .github/workflows/*.yml`
- `gh actions-lock --verify-local` (action の追加は無いが、lockfile の coverage を確認する)
- CI では `paths` job がテストを実行し、その後に実際の diff で `detect-tools.sh` を動かす。`paths` job 自体が失敗した場合も `gate` は `paths=failure` を見て失敗する

## 検討した代替案と捨てた理由

| 案 | 内容 | 捨てた理由 |
| --- | --- | --- |
| A. YAML 内の shell のまま | 判定を workflow に残す | 回帰テストを実行できず、非 ASCII の取りこぼしのような fail-open を CI で検出できない |
| B. `core.quotepath=false` だけを付ける | 行区切りのまま引用を止める | 改行を含む path では行が分かれ、別の path として判定される。NUL 区切りの方が path の表現に依存しない |
| C. 未知の result を成功として扱う | 従来どおり `failure` / `cancelled` だけを失敗にする | result の追加や取り違えが fail-open になる。許可するのは `success` と `skipped` に限定する |
| D. gate 集約の Lean モデルを追加 | 真理表を Lean で証明する | 状態遷移が無い有限な表で、4^5 を実装テストで全列挙できる。モデルを増やすと同じ表の二重管理になる |

## 影響

### 良い面

- 非 ASCII や改行を含む path でも tool の変更を取りこぼさない
- `paths` の選択と `gate` の集約を、YAML を読まずにローカルで検証できる
- `gate` の不変条件が真理表の全組み合わせで検査される

### 悪い面

- `gate` job でも checkout が要るため、集約の実行に repository の取得が 1 回増える
- `paths` job が script のテストも兼ねるため、テストが落ちると suite の選択より先に job が失敗する (gate は失敗するので fail-close)
