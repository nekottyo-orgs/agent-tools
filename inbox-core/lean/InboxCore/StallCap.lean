/-!
停止種別ごとの上限 T 分離 (nekottyo/agents#562) の仕様と性質。
F1 (unclaimed: 未着手・未受領) = 3 分、F2 (started: 着手済みだが無報告) = 15 分、
Mac sleep 除外 (`wokeAt`)、差し替え後の数え直し (`replacedAt`)、
1 回目の停止 → 差し替え (`replace`) / 2 回目の停止 → 人待ち (`move .stallCap .humanAction`)、
および単一上限 T との後方互換性を Nat の上で形式化する。

`inbox-core/src/lib.rs` (sha256 固定) には変更を加えず、`replace.rs` の
`StallKind` / `effective_cap_min` / `decide_for_kind` の純粋な仕様を表す。
Rust 実装との対応は `inbox-core/tests/stall_cap.rs` の差分テストで照合する。
-/
namespace InboxProofs.StallCap

/-- 停止種別。F1 (未着手・未受領) と F2 (着手済みだが無報告)。 -/
inductive StallKind where
  | unclaimed
  | started
  deriving DecidableEq, Repr

/-- F1 (未着手・未受領) の既定上限 (分)。 -/
def defaultUnclaimedCapMin : Nat := 3

/-- F2 (着手済みだが無報告) の既定上限 (分)。 -/
def defaultStartedCapMin : Nat := 15

/-- 停止種別から実効上限 (分) を選ぶ。 -/
def effectiveCap (kind : StallKind) (f1Cap f2Cap : Nat) : Nat :=
  match kind with
  | .unclaimed => f1Cap
  | .started => f2Cap

/-- Mac sleep 除外: 起きた時刻 `wokeAt` より前の検出は `wokeAt` から数え直す。 -/
def effectiveDetectedAt (detectedAt wokeAt : Nat) : Nat :=
  max detectedAt wokeAt

/-- 差し替え後の数え直し: 差し替え済み (`replacedAt > 0`) なら、それ以前の検出は無効。 -/
def effectiveDetected (detected : Bool) (detectedAt replacedAt : Nat) : Bool :=
  detected && (decide (replacedAt = 0) || decide (replacedAt < detectedAt))

inductive SLane where
  | todo
  | worker
  | humanDecision
  | humanAction
  | done
  deriving DecidableEq, Repr

inductive SPanes where
  | unknown
  | absent
  | closed
  | live
  deriving DecidableEq, Repr

inductive SRemaining where
  | empty
  | nothing
  | other
  deriving DecidableEq, Repr

inductive SRule where
  | settleGone
  | settleDone
  | paneGone
  | answered
  | waiting
  | stallCap
  | live
  deriving DecidableEq, Repr

inductive SVerdict where
  | stay
  | replace
  | move (rule : SRule) (dest : SLane)
  deriving DecidableEq, Repr

structure STaskFacts where
  panes : SPanes
  idle : Bool
  working : Bool
  closeFailed : Bool
  accepted : Bool
  reqAccepted : Bool
  remaining : SRemaining
  waiting : Bool
  answered : Bool
  detected : Bool
  detectedAt : Nat
  seen : Bool
  cleared : Bool
  deriving DecidableEq, Repr

structure SStopFacts where
  replacedAt : Nat
  wokeAt : Nat
  exhausted : Bool
  deriving DecidableEq, Repr

/-- 実効検出・Mac sleep 除外・停止種別別上限をすべて織り込んだ `over_cap` の仕様。 -/
def stallCapOver (f : STaskFacts) (s : SStopFacts) (now : Nat)
    (kind : StallKind) (f1Cap f2Cap : Nat) : Bool :=
  effectiveDetected f.detected f.detectedAt s.replacedAt
    && f.seen
    && !f.cleared
    && decide (effectiveDetectedAt f.detectedAt s.wokeAt + effectiveCap kind f1Cap f2Cap * 60 ≤ now)

def isTW : SLane → Bool
  | .todo | .worker => true
  | _ => false

def isLive : SPanes → Bool
  | .live => true
  | _ => false

def isGone : SPanes → Bool
  | .absent | .closed => true
  | _ => false

def isAbsent : SPanes → Bool
  | .absent => true
  | _ => false

def isNothing : SRemaining → Bool
  | .nothing => true
  | _ => false

def settleDest (rem : SRemaining) (waiting : Bool) : SLane :=
  match rem with
  | .other => if waiting then .humanDecision else .humanAction
  | _ => .done

/-- `lib.rs` の `next_lane` と同型の判定仕様 (`oc` は `stallCapOver` の値)。 -/
def nextLaneSpec (fr : SLane) (f : STaskFacts) (oc : Bool) : Option (SRule × SLane) :=
  if isTW fr && isGone f.panes && f.accepted then
    some (.settleGone, settleDest f.remaining f.waiting)
  else if isTW fr && isLive f.panes && f.idle && f.reqAccepted && !f.waiting && !f.closeFailed &&
      isNothing f.remaining then
    some (.settleDone, .done)
  else if (fr == .worker) && isAbsent f.panes then
    some (.paneGone, .todo)
  else if (fr == .humanDecision) && f.answered && !f.waiting then
    some (.answered, .todo)
  else if isTW fr && isLive f.panes && f.waiting then
    some (.waiting, .humanDecision)
  else if (fr == .worker) && isLive f.panes && f.idle && oc then
    some (.stallCap, .humanAction)
  else if (fr == .todo) && isLive f.panes && !(f.answered && !f.working) then
    some (.live, .worker)
  else
    none

/-- `replace::decide_for_kind` の仕様。 -/
def decideSpec (fr : SLane) (f : STaskFacts) (s : SStopFacts) (now : Nat)
    (kind : StallKind) (f1Cap f2Cap : Nat) : SVerdict :=
  let effDet := effectiveDetected f.detected f.detectedAt s.replacedAt
  let oc := stallCapOver f s now kind f1Cap f2Cap
  let first := decide (s.replacedAt = 0)
  match nextLaneSpec fr f oc with
  | some (.stallCap, dest) =>
    if first then .replace else .move .stallCap dest
  | some (r, dest) => .move r dest
  | none =>
    if first && s.exhausted && effDet && f.seen && !f.cleared &&
        (fr == .worker) && isLive f.panes && f.idle then
      .replace
    else
      .stay

/-! ## 基本性質 (a): 停止種別ごとの上限 -/

theorem effectiveCap_unclaimed (f1Cap f2Cap : Nat) :
    effectiveCap .unclaimed f1Cap f2Cap = f1Cap := rfl

theorem effectiveCap_started (f1Cap f2Cap : Nat) :
    effectiveCap .started f1Cap f2Cap = f2Cap := rfl

theorem default_f1_lt_f2 :
    effectiveCap .unclaimed defaultUnclaimedCapMin defaultStartedCapMin <
      effectiveCap .started defaultUnclaimedCapMin defaultStartedCapMin := by
  decide

/-! ## Mac sleep 除外 (b): `wokeAt` から実効上限を経過するまで発火しない -/

theorem sleep_excludes_over (f : STaskFacts) (s : SStopFacts) (now : Nat)
    (kind : StallKind) (f1Cap f2Cap : Nat)
    (hsleep : now < s.wokeAt + effectiveCap kind f1Cap f2Cap * 60) :
    stallCapOver f s now kind f1Cap f2Cap = false := by
  unfold stallCapOver effectiveDetectedAt
  have hnot : ¬ (max f.detectedAt s.wokeAt + effectiveCap kind f1Cap f2Cap * 60 ≤ now) := by
    have hle : s.wokeAt ≤ max f.detectedAt s.wokeAt := Nat.le_max_right _ _
    omega
  simp [hnot]

private theorem nextLaneSpec_ne_stallCap_false (fr : SLane) (f : STaskFacts) (dest : SLane) :
    nextLaneSpec fr f false ≠ some (.stallCap, dest) := by
  unfold nextLaneSpec
  simp only [Bool.and_false, Bool.false_eq_true, ↓reduceIte]
  repeat (first | split | simp)

theorem sleep_excludes_move (fr : SLane) (f : STaskFacts) (s : SStopFacts) (now : Nat)
    (kind : StallKind) (f1Cap f2Cap : Nat)
    (hsleep : now < s.wokeAt + effectiveCap kind f1Cap f2Cap * 60) :
    decideSpec fr f s now kind f1Cap f2Cap ≠ .move .stallCap .humanAction := by
  have hoc : stallCapOver f s now kind f1Cap f2Cap = false :=
    sleep_excludes_over f s now kind f1Cap f2Cap hsleep
  unfold decideSpec
  dsimp only
  rw [hoc]
  cases h : nextLaneSpec fr f false with
  | none =>
    simp only
    split <;> simp
  | some p =>
    rcases p with ⟨r, d⟩
    cases r with
    | stallCap => exact absurd h (nextLaneSpec_ne_stallCap_false fr f d)
    | settleGone | settleDone | paneGone | answered | waiting | live => simp

theorem sleep_excludes_replace (fr : SLane) (f : STaskFacts) (s : SStopFacts) (now : Nat)
    (kind : StallKind) (f1Cap f2Cap : Nat)
    (hnotex : s.exhausted = false)
    (hsleep : now < s.wokeAt + effectiveCap kind f1Cap f2Cap * 60) :
    decideSpec fr f s now kind f1Cap f2Cap ≠ .replace := by
  have hoc : stallCapOver f s now kind f1Cap f2Cap = false :=
    sleep_excludes_over f s now kind f1Cap f2Cap hsleep
  unfold decideSpec
  dsimp only
  rw [hoc, hnotex]
  cases h : nextLaneSpec fr f false with
  | none => simp
  | some p =>
    rcases p with ⟨r, d⟩
    cases r with
    | stallCap => exact absurd h (nextLaneSpec_ne_stallCap_false fr f d)
    | settleGone | settleDone | paneGone | answered | waiting | live => simp

/-! ## 差し替え後の数え直し (c): `replacedAt` 以前の検出は無効 -/

theorem recount_ignores_old (detected : Bool) (detectedAt replacedAt : Nat)
    (hrep : 0 < replacedAt) (hold : detectedAt ≤ replacedAt) :
    effectiveDetected detected detectedAt replacedAt = false := by
  unfold effectiveDetected
  have h1 : replacedAt ≠ 0 := by omega
  have h2 : ¬ (replacedAt < detectedAt) := by omega
  simp [h1, h2]

theorem recount_blocks_over (f : STaskFacts) (s : SStopFacts) (now : Nat)
    (kind : StallKind) (f1Cap f2Cap : Nat)
    (hrep : 0 < s.replacedAt) (hold : f.detectedAt ≤ s.replacedAt) :
    stallCapOver f s now kind f1Cap f2Cap = false := by
  unfold stallCapOver
  rw [recount_ignores_old f.detected f.detectedAt s.replacedAt hrep hold]
  rfl

theorem recount_blocks_stall (fr : SLane) (f : STaskFacts) (s : SStopFacts)
    (now : Nat) (kind : StallKind) (f1Cap f2Cap : Nat)
    (hrep : 0 < s.replacedAt) (hold : f.detectedAt ≤ s.replacedAt) :
    decideSpec fr f s now kind f1Cap f2Cap ≠ .replace ∧
      decideSpec fr f s now kind f1Cap f2Cap ≠ .move .stallCap .humanAction := by
  have hoc : stallCapOver f s now kind f1Cap f2Cap = false :=
    recount_blocks_over f s now kind f1Cap f2Cap hrep hold
  have hne0 : s.replacedAt ≠ 0 := by omega
  unfold decideSpec
  dsimp only
  rw [hoc]
  cases h : nextLaneSpec fr f false with
  | none => simp [hne0]
  | some p =>
    rcases p with ⟨r, d⟩
    cases r with
    | stallCap => exact absurd h (nextLaneSpec_ne_stallCap_false fr f d)
    | settleGone | settleDone | paneGone | answered | waiting | live => simp

/-! ## 1 回目と 2 回目の停止 (d): 1 回目は `replace`、2 回目は `humanAction` (二度目の差し替え無し) -/

theorem first_stall_replaces (fr : SLane) (f : STaskFacts) (s : SStopFacts) (now : Nat)
    (kind : StallKind) (f1Cap f2Cap : Nat)
    (hfirst : s.replacedAt = 0)
    (hstall : nextLaneSpec fr f (stallCapOver f s now kind f1Cap f2Cap) = some (.stallCap, .humanAction)) :
    decideSpec fr f s now kind f1Cap f2Cap = .replace := by
  unfold decideSpec
  simp [hfirst, hstall]

theorem second_stall_human (fr : SLane) (f : STaskFacts) (s : SStopFacts)
    (now : Nat) (kind : StallKind) (f1Cap f2Cap : Nat)
    (hrep : 0 < s.replacedAt)
    (hstall : nextLaneSpec fr f (stallCapOver f s now kind f1Cap f2Cap) = some (.stallCap, .humanAction)) :
    decideSpec fr f s now kind f1Cap f2Cap = .move .stallCap .humanAction := by
  have hne0 : s.replacedAt ≠ 0 := by omega
  unfold decideSpec
  simp [hne0, hstall]

theorem no_second_replace (fr : SLane) (f : STaskFacts) (s : SStopFacts) (now : Nat)
    (kind : StallKind) (f1Cap f2Cap : Nat)
    (hrep : 0 < s.replacedAt) :
    decideSpec fr f s now kind f1Cap f2Cap ≠ .replace := by
  have hne0 : s.replacedAt ≠ 0 := by omega
  unfold decideSpec
  dsimp only
  cases h : nextLaneSpec fr f (stallCapOver f s now kind f1Cap f2Cap) with
  | none => simp [hne0]
  | some p =>
    rcases p with ⟨r, d⟩
    cases r <;> simp [hne0]

/-! ## 後方互換性 (e): `f1Cap = f2Cap = cap` なら単一上限 `cap` と一致 -/

theorem legacy_cap_compat (kind : StallKind) (cap : Nat) :
    effectiveCap kind cap cap = cap := by
  cases kind <;> rfl

theorem legacy_decide_compat (fr : SLane) (f : STaskFacts) (s : SStopFacts) (now : Nat)
    (kind : StallKind) (cap : Nat) :
    decideSpec fr f s now kind cap cap = decideSpec fr f s now .unclaimed cap cap := by
  unfold decideSpec stallCapOver
  rw [legacy_cap_compat kind cap, legacy_cap_compat .unclaimed cap]

/-! ## F1 / F2 の分離: F1 は F2 より早く上限に達する -/

theorem f1_before_f2 (f : STaskFacts) (s : SStopFacts) (now f1Cap f2Cap : Nat)
    (hlt : f1Cap < f2Cap)
    (hdet : effectiveDetected f.detected f.detectedAt s.replacedAt = true)
    (hseen : f.seen = true)
    (hclr : f.cleared = false)
    (hnow : now = effectiveDetectedAt f.detectedAt s.wokeAt + f1Cap * 60) :
    stallCapOver f s now .unclaimed f1Cap f2Cap = true ∧
      stallCapOver f s now .started f1Cap f2Cap = false := by
  unfold stallCapOver
  simp [hdet, hseen, hclr, effectiveCap, hnow]
  omega

theorem default_f1_at_3min (f : STaskFacts) (s : SStopFacts)
    (hdet : effectiveDetected f.detected f.detectedAt s.replacedAt = true)
    (hseen : f.seen = true)
    (hclr : f.cleared = false) :
    let now := effectiveDetectedAt f.detectedAt s.wokeAt + 180
    stallCapOver f s now .unclaimed defaultUnclaimedCapMin defaultStartedCapMin = true ∧
      stallCapOver f s now .started defaultUnclaimedCapMin defaultStartedCapMin = false := by
  exact f1_before_f2 f s
    (effectiveDetectedAt f.detectedAt s.wokeAt + defaultUnclaimedCapMin * 60)
    defaultUnclaimedCapMin defaultStartedCapMin
    default_f1_lt_f2 hdet hseen hclr rfl

end InboxProofs.StallCap

#print axioms InboxProofs.StallCap.sleep_excludes_over
#print axioms InboxProofs.StallCap.sleep_excludes_move
#print axioms InboxProofs.StallCap.sleep_excludes_replace
#print axioms InboxProofs.StallCap.recount_ignores_old
#print axioms InboxProofs.StallCap.recount_blocks_over
#print axioms InboxProofs.StallCap.recount_blocks_stall
#print axioms InboxProofs.StallCap.first_stall_replaces
#print axioms InboxProofs.StallCap.second_stall_human
#print axioms InboxProofs.StallCap.no_second_replace
#print axioms InboxProofs.StallCap.legacy_decide_compat
#print axioms InboxProofs.StallCap.f1_before_f2
#print axioms InboxProofs.StallCap.default_f1_at_3min
