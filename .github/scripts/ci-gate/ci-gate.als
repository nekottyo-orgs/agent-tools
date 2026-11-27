// ci-gate の集約判定 (.github/scripts/ci-gate/check-results.sh) のモデル。
//
// 対応 (correspondence):
//   - Name は check-results.sh の required_jobs と、そこに無い job 名 (UnknownName) を表す。
//   - Result は success / skipped / failure / cancelled と、それ以外の result (UnknownResult) を表す。
//   - Entry は `job名=result` の 1 引数。Input.args は引数の並び (重複を区別するため seq)。
//   - scriptGate は check-results.sh の判定をそのまま写す。引数が 1 つ以上あり、未知の job 名が
//     無く、success / skipped 以外の result が無く、job 名の重複が無く、必須 job がすべて現れる
//     ときだけ成功する。
//   - expected は「必須 job がちょうど揃い、すべて success か skipped」という仕様。
//
// 抽象化の範囲 (why):
//   - `=` の無い引数、空の job 名、空の result は、いずれも check-results.sh が失敗にする。
//     モデルでは UnknownName / UnknownResult を伴う Entry として扱う。gate の成否は同じ失敗に
//     なるので、この 3 つを区別しない。形式ごとの扱いは ci-gate.test.sh が確かめる。
//   - 空の引数列は Input.args.elems が空になる。check-results.sh の「引数が 0 個」と同じ。
//   - 未知の job 名と result は、必須集合・既知集合への所属だけを見るため、種類ごとに 1 atom で足りる。
//   - 引数の順序は成否に影響しない。seq を使うのは重複を区別するためだけ。
//
// 検証: repo root から (Alloy 6.2.0 で確認)
//   java -jar <alloy.jar> exec -f -c Correspondence -t text -o /tmp/ci-gate-alloy \
//     .github/scripts/ci-gate/ci-gate.als
// check は scope 内の反例探索 (有限の全列挙ではない)。scope を 8 にしているのは、必須 5 job に
// 重複や未知の job を足した反例 (最大 7 引数) まで含めるため。

module ciGate

// check-results.sh の required_jobs と同じ並び・同じ 5 つ
abstract sig Name {}
one sig Paths, BashGates, HerdrTimeline, InboxCore, SecretScan, UnknownName extends Name {}

fun Required : set Name {
  Paths + BashGates + HerdrTimeline + InboxCore + SecretScan
}

abstract sig Result {}
one sig Success, Skipped, Failure, Cancelled, UnknownResult extends Result {}

fun Passing : set Result { Success + Skipped }

sig Entry {
  name : one Name,
  result : one Result
}

one sig Input {
  // 引数の並び。重複した job 名を別の Entry として持てるように seq にする
  args : seq Entry
}

// check-results.sh の重複判定: 同じ job 名が 2 回以上現れない
pred noDuplicates {
  all n : Name | lone (Input.args.elems & name.n)
}

// check-results.sh の判定。成功なら終了コード 0
pred scriptGate {
  some Input.args.elems
  no (Input.args.elems.name & UnknownName)
  no (Input.args.elems.result - Passing)
  noDuplicates
  Required in Input.args.elems.name
}

// 仕様: 必須 job がちょうど揃い、すべて success か skipped
pred expected {
  Input.args.elems.name = Required
  #Input.args.elems = #Required
  no (Input.args.elems.result - Passing)
}

// 集約ルールの性質: gate が成功するのは、必須 job がちょうど揃い、
// すべて success か skipped のとき、かつそのときに限る
assert Correspondence {
  scriptGate iff expected
}

check Correspondence for 8

// 成功する入力が存在すること (check が空の scope で自明に通っていないことの確認)
pred showPass { expected }
run showPass for 8
