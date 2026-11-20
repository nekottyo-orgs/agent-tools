/-
窓口まわりの受け渡しの状態機械。
登場するもの: 人 (answer / 発言)、Cue (cue: SQLite の decisions)、Cue worker (Cue 専任。claim と token)、
窓口 (add / ack / reply / hold / 張り直し)、worker (send)、inbox (msgs の box: new / held / cur)。

修正前・移行期・修正後は同じ State・同じ操作・同じ故障を持ち、遷移の guard だけを Config の flag で変える。
`e4_of` / `e5_of` のように flag を仮定に取る定理では、その flag が性質の依存する release / PR を表す。
-/
namespace Handoff

def upd {α β : Type} [DecidableEq α] (f : α → β) (a : α) (b : β) : α → β :=
  fun x => if x = a then b else f x

@[simp] theorem upd_same {α β : Type} [DecidableEq α] (f : α → β) (a : α) (b : β) :
    upd f a b a = b := by simp [upd]

theorem upd_apply {α β : Type} [DecidableEq α] (f : α → β) (a : α) (b : β) (x : α) :
    upd f a b x = if x = a then b else f x := rfl

abbrev DId := Nat
abbrev MId := Nat
abbrev Tok := Nat
abbrev Pane := Nat

/-- request_key の形。own = frontdesk-<自 run>/...、oldFormat = frontdesk/... や frontdesk-<YYYYMMDD>/...、
missing = key 無し、other = 窓口以外の用途の key -/
inductive Key | own | oldFormat | missing | other
  deriving DecidableEq, Repr

/-- Cue への登録の経路 -/
inductive Route
  /-- decision-inbox-add.sh (接頭辞を組み立てる) -/
  | script
  /-- CLI を素のパスで呼ぶ (guard の ANCHOR が当たる) -/
  | rawPlain
  /-- CLI を引用符付きのパスで呼ぶ (REPL の $ が '...' で包む。guard の ANCHOR が外れる) -/
  | rawQuoted
  /-- POST /api/decisions (PR 12 の pre-add の対象外) -/
  | http
  deriving DecidableEq, Repr

structure Dec where
  /-- request_key が frontdesk-<自 run>/ で始まる。Cue worker の claim --key-prefix と検知はこれで絞る -/
  prefixed : Bool
  /-- 窓口か Cue worker の pane から登録された (取りこぼしを問う対象) -/
  byFd : Bool
  answered : Bool
  claim : Option Tok
  done : Bool
  deriving DecidableEq, Repr

inductive Tag | escalation | request | alert | completion | progress | reply
  deriving DecidableEq, Repr

/-- run-inbox.sh の needs_reply。返信不要のタグを列挙し、それ以外は返信必須 -/
def Tag.needsReply : Tag → Bool
  | .completion | .progress | .reply => false
  | _ => true

inductive Box | new | held | cur
  deriving DecidableEq, Repr

/-- cur へ移ったときの扱い -/
inductive Res | none | replied | noReply | senderGone
  deriving DecidableEq, Repr

structure Msg where
  tag : Tag
  sender : Pane
  box : Box
  res : Res
  /-- 窓口の目に入った (通知・張り直し・人の発言の注入のどれか) -/
  surfaced : Bool
  /-- 今張っている watch の seen に入っている (--new-only) -/
  skipped : Bool
  deriving DecidableEq, Repr

structure State where
  cue : DId → Option Dec
  cueWorkerAlive : Bool
  /-- Cue worker の process か出力 (stdout / REPL の変数) にだけある token -/
  mem : DId → Option Tok
  /-- token ファイル。名前は claim ごとの乱数なので token で引く -/
  file : Tok → Option DId
  /-- 保存先を開ける (gate・git 管理下・権限で止まっていない) -/
  openable : Bool
  /-- 途中の claim。script: claim を commit したが出力先へ書き終えていない / tx: ファイルを公開したが commit していない -/
  pending : Option (DId × Tok)
  msgs : MId → Option Msg
  /-- 窓口の受信待ち (wait-first-line.sh で包んだ watch) が生きている -/
  armed : Bool
  senderAlive : Pane → Bool
  /-- 回答済みで未 claim の判断が検知されて人か窓口に見えた -/
  detected : DId → Bool
  /-- 送り元に [reply] を送れた -/
  replied : MId → Bool
  /-- state file に写した判断の一覧 (回答済みか) -/
  snapshot : DId → Option Bool

def init : State where
  cue := fun _ => none
  cueWorkerAlive := true
  mem := fun _ => none
  file := fun _ => none
  openable := true
  pending := none
  msgs := fun _ => none
  armed := false
  senderAlive := fun _ => true
  detected := fun _ => false
  replied := fun _ => false
  snapshot := fun _ => none

/-- 設計の flag。どれも修正前は false -/
structure Config where
  /-- CLI の add が pre-add (cue-pre-add.sh) を呼ぶ (decision-inbox の release 1 回目) -/
  preAdd : Bool
  /-- claim --token-file: ファイルを公開してから commit する (release 1 回目) -/
  tokenFileTx : Bool
  /-- cue-claim.sh --out: 出力先を開いてから claim を呼ぶ (移行期) -/
  claimScript : Bool
  /-- ack は返信必須の msg を断り、reply は送れてから cur へ移す (ADR-316) -/
  replyGate : Bool
  /-- 張り直しで new/ に残る報告を出して watch を張らない (PR 318) -/
  rearmSurface : Bool
  /-- 人の発言で new/ に残る報告を出す (PR 318) -/
  humanSurface : Bool
  /-- 回答済みで未 claim の判断の検知 (w7W の別 PR) -/
  detector : Bool
  /-- 復旧通知が Cue をその場で引く (状態の一元化、段階 0) -/
  derivedRecovery : Bool

def Config.pre : Config :=
  { preAdd := false, tokenFileTx := false, claimScript := false, replyGate := false,
    rearmSurface := false, humanSurface := false, detector := false, derivedRecovery := false }

/-- 今 (この PR + PR 318)。release と w7W の PR の前 -/
def Config.stage0 : Config :=
  { preAdd := false, tokenFileTx := false, claimScript := true, replyGate := true,
    rearmSurface := true, humanSurface := true, detector := false, derivedRecovery := true }

/-- 設計書の採った形 + 追加要件の段階 1 まで -/
def Config.post : Config :=
  { preAdd := true, tokenFileTx := true, claimScript := false, replyGate := true,
    rearmSurface := true, humanSurface := true, detector := true, derivedRecovery := true }

/-- 環境 (時間を通して固定) -/
structure World where
  /-- 実際に窓口か Cue worker として動いている pane -/
  fd : Pane → Bool
  /-- guard / pre-add が窓口か Cue worker と識別できる pane
  (run.md に orchestrator_pane / cue_worker_pane として載り、HERDR_PANE_ID があり、hook が配線済み) -/
  reg : Pane → Bool
  /-- 窓口か Cue worker が HTTP で登録することがある -/
  httpFromFd : Bool

/-- cue-pre-add.sh の規則 1 と 2 -/
def PreAddOk (w : World) (p : Pane) (k : Key) : Prop :=
  k ≠ .oldFormat ∧ (w.reg p = true → k = .own)

/-- cue-add-prefix-guard.sh (字面の照合。rawPlain にしか当たらない) -/
def GuardOk (w : World) (p : Pane) (k : Key) : Prop :=
  w.reg p = true → k = .own

def Allowed (c : Config) (w : World) (p : Pane) (k : Key) : Route → Prop
  | .script => k = .own
  | .rawPlain => GuardOk w p k ∧ (c.preAdd = true → PreAddOk w p k)
  | .rawQuoted => c.preAdd = true → PreAddOk w p k
  | .http => w.fd p = true → w.httpFromFd = true

/-- Cue worker が claim できる (claim --key-prefix frontdesk-<run>/ で絞る) -/
def Claimable (s : State) (d : DId) (x : Dec) : Prop :=
  s.cue d = some x ∧ x.prefixed = true ∧ x.answered = true ∧ x.claim = none ∧
    x.done = false ∧ s.cueWorkerAlive = true

def mapMsgs (f : Msg → Msg) (ms : MId → Option Msg) : MId → Option Msg :=
  fun i => (ms i).map f

/-- new/ にあるものを窓口に出す -/
def showNew (ms : MId → Option Msg) := mapMsgs (fun x => if x.box = .new then { x with surfaced := true } else x) ms
/-- watch の通知: new/ にあって seen に無いもの -/
def fireNew (ms : MId → Option Msg) :=
  mapMsgs (fun x => if x.box = .new ∧ x.skipped = false then { x with surfaced := true } else x) ms
/-- watch --new-only: 起動時の new/ を seen に入れる -/
def skipNew (ms : MId → Option Msg) := mapMsgs (fun x => if x.box = .new then { x with skipped := true } else x) ms
/-- --new-only なしの watch: seen は空から始まる -/
def unskip (ms : MId → Option Msg) := mapMsgs (fun x => { x with skipped := false }) ms

def HasNew (s : State) : Prop := ∃ m x, s.msgs m = some x ∧ x.box = .new

def toCur (x : Msg) (r : Res) : Msg := { x with box := .cur, res := r }

/-- 故障。設計の flag に依らず、どの設定でも起きる -/
inductive Fault : State → State → Prop
  /-- Cue worker の process の消失 (session の消失、保持 process の終了) -/
  | crash (s) : Fault s { s with cueWorkerAlive := false, mem := fun _ => none, pending := none }
  /-- 出力の切断 (heredoc の EOF、出力を切った、REPL の 600 秒で background へ) -/
  | outputCut (s) : Fault s { s with mem := fun _ => none }
  /-- 保存先が開けなくなる (delegation-gate が Write を止めた、等) -/
  | blockSave (s) : Fault s { s with openable := false }
  /-- 受信待ちの停止 (watch の process 消失、時間切れ) -/
  | watchDie (s) : Fault s { s with armed := false }
  /-- 送り元の pane の消失 -/
  | senderDie (s) (p : Pane) : Fault s { s with senderAlive := upd s.senderAlive p false }

/-- 窓口の受信待ちの張り直し -/
inductive Rearm (c : Config) : State → State → Prop
  | reveal (s) : c.rearmSurface = true → HasNew s → Rearm c s { s with msgs := showNew s.msgs }
  | arm (s) (newOnly : Bool) : ¬ (c.rearmSurface = true ∧ HasNew s) →
      Rearm c s { s with armed := true, msgs := if newOnly then skipNew s.msgs else unskip s.msgs }

/-- 人の発言 (UserPromptSubmit の注入) -/
inductive Human (c : Config) : State → State → Prop
  | speak (s) : Human c s (if c.humanSurface = true then { s with msgs := showNew s.msgs } else s)

def newDec (w : World) (p : Pane) (k : Key) : Dec :=
  { prefixed := decide (k = .own), byFd := w.fd p, answered := false, claim := none, done := false }

def newMsg (t : Tag) (p : Pane) : Msg :=
  { tag := t, sender := p, box := .new, res := .none, surfaced := false, skipped := false }

inductive Step (c : Config) (w : World) : State → State → Prop
  | fault {s s'} : Fault s s' → Step c w s s'
  | rearm {s s'} : Rearm c s s' → Step c w s s'
  | human {s s'} : Human c s s' → Step c w s s'
  | restart (s) : Step c w s { s with cueWorkerAlive := true }
  | unblockSave (s) : Step c w s { s with openable := true }
  -- 登録 (add / pre-add)
  | add (s) (p : Pane) (d : DId) (k : Key) (r : Route) :
      s.cue d = none → Allowed c w p k r →
      Step c w s { s with cue := upd s.cue d (some (newDec w p k)) }
  -- 人の回答 (Cue は二重回答を拒む)
  | answer (s) (d : DId) (x : Dec) : s.cue d = some x → x.answered = false →
      Step c w s { s with cue := upd s.cue d (some { x with answered := true }) }
  -- 修正前の claim: token は stdout に出て、保存は別の操作
  | claimStdout (s) (d : DId) (x : Dec) (t : Tok) :
      c.tokenFileTx = false → c.claimScript = false → Claimable s d x →
      Step c w s { s with cue := upd s.cue d (some { x with claim := some t }),
                          mem := upd s.mem d (some t) }
  | save (s) (d : DId) (t : Tok) :
      c.tokenFileTx = false → c.claimScript = false → s.mem d = some t → s.openable = true →
      Step c w s { s with file := upd s.file t (some d) }
  -- 移行期の cue-claim.sh: 出力先を開いてから claim。claim の commit の後に stdout (=ファイル) へ書く
  | scriptClaim (s) (d : DId) (x : Dec) (t : Tok) :
      c.claimScript = true → Claimable s d x → s.openable = true → s.pending = none → s.file t = none →
      Step c w s { s with cue := upd s.cue d (some { x with claim := some t }), pending := some (d, t) }
  | scriptFlush (s) (d : DId) (t : Tok) : c.claimScript = true → s.pending = some (d, t) →
      Step c w s { s with file := upd s.file t (some d), pending := none }
  -- 修正後の claim --token-file: transaction の中でファイルを公開してから commit
  | txPublish (s) (d : DId) (x : Dec) (t : Tok) :
      c.tokenFileTx = true → Claimable s d x → s.openable = true → s.pending = none → s.file t = none →
      Step c w s { s with file := upd s.file t (some d), pending := some (d, t) }
  | txCommit (s) (d : DId) (x : Dec) (t : Tok) :
      c.tokenFileTx = true → s.pending = some (d, t) → s.cue d = some x → x.claim = none →
      Step c w s { s with cue := upd s.cue d (some { x with claim := some t }), pending := none }
  | complete (s) (d : DId) (x : Dec) (t : Tok) :
      s.cue d = some x → x.claim = some t → (s.mem d = some t ∨ s.file t = some d) →
      Step c w s { s with cue := upd s.cue d (some { x with claim := none, done := true }),
                          file := upd s.file t none }
  | expire (s) (d : DId) (x : Dec) (t : Tok) : s.cue d = some x → x.claim = some t →
      Step c w s { s with cue := upd s.cue d (some { x with claim := none }) }
  -- 検知 (frontdesk-health。DB を自 run の接頭辞で読む)
  | detect (s) (d : DId) (x : Dec) :
      c.detector = true → s.cue d = some x → x.prefixed = true → x.answered = true →
      x.claim = none → x.done = false →
      Step c w s { s with detected := upd s.detected d true }
  -- state file への一覧の写し
  | snapshot (s) : Step c w s { s with snapshot := fun d => (s.cue d).map (·.answered) }
  -- inbox
  | send (s) (m : MId) (t : Tag) (p : Pane) : s.msgs m = none →
      Step c w s { s with msgs := upd s.msgs m (some (newMsg t p)) }
  | fire (s) : s.armed = true → Step c w s { s with armed := false, msgs := fireNew s.msgs }
  | ack (s) (m : MId) (x : Msg) : s.msgs m = some x → x.box ≠ .cur →
      (c.replyGate = true → x.tag.needsReply = false) →
      Step c w s { s with msgs := upd s.msgs m (some (toCur x .none)) }
  | ackNoReply (s) (m : MId) (x : Msg) : s.msgs m = some x → x.box ≠ .cur →
      (c.replyGate = true → x.tag ≠ .escalation ∧ x.tag ≠ .request) →
      Step c w s { s with msgs := upd s.msgs m (some (toCur x .noReply)) }
  | ackSenderGone (s) (m : MId) (x : Msg) : s.msgs m = some x → x.box ≠ .cur →
      s.senderAlive x.sender = false →
      Step c w s { s with msgs := upd s.msgs m (some (toCur x .senderGone)) }
  /-- reply: 送れたときだけ cur へ移す (修正前は手で送って手で ack したので、この 1 操作は修正前にも残してある) -/
  | reply (s) (m : MId) (x : Msg) : s.msgs m = some x → x.box ≠ .cur → s.senderAlive x.sender = true →
      Step c w s { s with msgs := upd s.msgs m (some (toCur x .replied)), replied := upd s.replied m true }
  | hold (s) (m : MId) (x : Msg) : s.msgs m = some x → x.box = .new →
      Step c w s { s with msgs := upd s.msgs m (some { x with box := .held }) }

inductive Reach (c : Config) (w : World) : State → Prop
  | init : Reach c w init
  | step {s s'} : Reach c w s → Step c w s s' → Reach c w s'

/-- 復旧通知が見せる判断の一覧 -/
def recovery (c : Config) (s : State) : DId → Option Bool :=
  if c.derivedRecovery = true then fun d => (s.cue d).map (·.answered) else s.snapshot

/-! ## 性質。どれも Config と World を引数に取り、修正後で証明し修正前で反証する -/

/-- 接頭辞: 窓口か Cue worker からの登録は、どの経路でも自 run の接頭辞を持つ -/
def PrefixProp (c : Config) (w : World) : Prop :=
  ∀ s, Reach c w s → ∀ d x, s.cue d = some x → x.byFd = true → x.prefixed = true

/-- E2: 回答済みで未 claim の判断は、次の 1 歩で claim されうるか検知されうる -/
def E2Prop (c : Config) (w : World) : Prop :=
  ∀ s, Reach c w s → ∀ d x, s.cue d = some x → x.byFd = true → x.answered = true →
    x.claim = none → x.done = false →
    ∃ s', Step c w s s' ∧ (s'.detected d = true ∨ ∃ y, s'.cue d = some y ∧ y.claim.isSome)

/-- E7: 有効な claim の token は、ファイルか process のどちらかに必ずある -/
def E7Prop (c : Config) (w : World) : Prop :=
  ∀ s, Reach c w s → ∀ d x t, s.cue d = some x → x.claim = some t →
    s.file t = some d ∨ s.mem d = some t

def Resolved (s : State) (m : MId) (x : Msg) : Prop :=
  (x.res = .replied ∧ s.replied m = true) ∨ (x.tag = .alert ∧ x.res = .noReply) ∨ x.res = .senderGone

/-- E4 / E6: 返信必須の msg は、reply か許された no-reply を経ないと cur に移らない -/
def E4Prop (c : Config) (w : World) : Prop :=
  ∀ s, Reach c w s → ∀ m x, s.msgs m = some x → x.box = .cur → x.tag.needsReply = true → Resolved s m x

/-- E5: new/ に残る報告は、次の張り直しでも人の発言でも窓口に出る -/
def E5Prop (c : Config) (w : World) : Prop :=
  ∀ s, Reach c w s → ∀ m x, s.msgs m = some x → x.box = .new →
    (∀ s', Rearm c s s' → ∃ y, s'.msgs m = some y ∧ y.surfaced = true) ∧
    (∀ s', Human c s s' → ∃ y, s'.msgs m = some y ∧ y.surfaced = true)

/-- 一元化: 復旧通知の一覧は Cue と一致する -/
def RecoveryProp (c : Config) (w : World) : Prop :=
  ∀ s, Reach c w s → recovery c s = fun d => (s.cue d).map (·.answered)

end Handoff
