import Aeneas
import InboxCore.InboxCore
import InboxCore.Stalled
open Aeneas Std Result

/-! Aeneas が生成した inbox_core.resend_step (再送の段) と inbox_core.next_lane (lane の照合) についての定理 -/
namespace InboxProofs

/-! ## resend_step -/

theorem resend_step_total (f : inbox_core.ResendFacts) : ∃ r, inbox_core.resend_step f = ok r := by
  rcases f with ⟨tr, ln, ns, bl, ag, eu, inp, own, pa, dl, od⟩
  cases tr
  · cases ag <;> cases eu <;> cases inp <;> cases pa <;> cases own <;> cases dl <;>
      simp [inbox_core.resend_step]
  · cases ln <;> cases ns <;> cases bl <;> cases ag <;> cases od <;> cases inp <;>
      simp [inbox_core.resend_step]

/-- 段 1 (Enter) は、段 1 を未使用で、Claude の入力欄に自分の本文が貼り付けでなく残り、まだ実行されていないときだけ -/
theorem resend_enter_iff (f : inbox_core.ResendFacts) :
    inbox_core.resend_step f = ok .Enter ↔
      f.tried = false ∧ f.agent = .Claude ∧ f.enter_used = false ∧ f.input = .Draft ∧
        f.pasted = false ∧ f.own = true ∧ f.delivered = .Undelivered := by
  rcases f with ⟨tr, ln, ns, bl, ag, eu, inp, own, pa, dl, od⟩
  cases tr <;> cases ag <;> cases eu <;> cases inp <;> cases own <;> cases pa <;> cases dl <;>
    simp [inbox_core.resend_step] <;> try (split_ifs <;> simp)

theorem resend_nudge_iff (f : inbox_core.ResendFacts) :
    inbox_core.resend_step f = ok .Nudge ↔
      f.tried = true ∧ f.last_nudge = false ∧ f.nudge_sent = false ∧ f.blocked = false ∧
        f.agent = .Claude ∧ f.overdue = false ∧ f.input = .Empty := by
  rcases f with ⟨tr, ln, ns, bl, ag, eu, inp, own, pa, dl, od⟩
  cases tr
  · cases ag <;> cases eu <;> cases inp <;> cases pa <;> cases own <;> cases dl <;>
      simp [inbox_core.resend_step]
  · cases ln <;> cases ns <;> cases bl <;> cases ag <;> cases od <;> cases inp <;>
      simp [inbox_core.resend_step]

theorem resend_exhausted_iff (f : inbox_core.ResendFacts) :
    inbox_core.resend_step f = ok .Exhausted ↔
      f.tried = true ∧ (f.last_nudge = true ∨ f.nudge_sent = true ∨ f.blocked = true) := by
  rcases f with ⟨tr, ln, ns, bl, ag, eu, inp, own, pa, dl, od⟩
  cases tr
  · cases ag <;> cases eu <;> cases inp <;> cases pa <;> cases own <;> cases dl <;>
      simp [inbox_core.resend_step]
  · cases ln <;> cases ns <;> cases bl <;> cases ag <;> cases od <;> cases inp <;>
      simp [inbox_core.resend_step]

/-- D7: 上限 T を越えた最後の 1 通には催促しない -/
theorem resend_overdue_no_nudge (f : inbox_core.ResendFacts) (h : f.overdue = true) :
    inbox_core.resend_step f ≠ ok .Nudge := by
  rw [Ne, resend_nudge_iff]; simp [h]

/-- 外殻が tried = true を渡せば Enter を返さない。段 1 を使ったことを台帳に残し、次の判定で tried に読み直すのは外殻で、
1 通に 1 回になるかはこの定理の外 -/
theorem resend_tried_no_enter (f : inbox_core.ResendFacts) (h : f.tried = true) :
    inbox_core.resend_step f ≠ ok .Enter := by
  rw [Ne, resend_enter_iff]; simp [h]

/-! ## next_lane

生成物を直接場合分けすると枝が多いので、Bool と Option の上の仕様 laneSpec を置き、生成物がそれを返すことを先に示す。
over_cap の値は引数 oc に括り出す。収束などの性質は laneSpec の上で示す -/

def isTW : inbox_core.Lane → Bool
  | .Todo | .Worker => true
  | _ => false

def isLive : inbox_core.Panes → Bool
  | .Live => true
  | _ => false

def isGone : inbox_core.Panes → Bool
  | .Absent | .Closed => true
  | _ => false

def isTodo : inbox_core.Lane → Bool
  | .Todo => true
  | _ => false

def isWorker : inbox_core.Lane → Bool
  | .Worker => true
  | _ => false

def isHD : inbox_core.Lane → Bool
  | .HumanDecision => true
  | _ => false

def isAbsent : inbox_core.Panes → Bool
  | .Absent => true
  | _ => false

def isNothing : inbox_core.Remaining → Bool
  | .Nothing => true
  | _ => false

/-- SettleGone の行き先。残りが無ければ done、あれば判断待ちかどうかで分ける -/
def settleDest (rem : inbox_core.Remaining) (waiting : Bool) : inbox_core.Lane :=
  match rem with
  | .Other => if waiting then .HumanDecision else .HumanAction
  | _ => .Done

/-- lib.rs の next_lane の判定を上から順に並べたもの。oc は over_cap の値 -/
def laneSpec (fr : inbox_core.Lane) (f : inbox_core.TaskFacts) (oc : Bool) :
    Option (inbox_core.Rule × inbox_core.Lane) :=
  if isTW fr && isGone f.panes && f.accepted then some (.SettleGone, settleDest f.remaining f.waiting)
  else if isTW fr && isLive f.panes && f.idle && f.req_accepted && !f.waiting && !f.close_failed &&
      isNothing f.remaining then some (.SettleDone, .Done)
  else if isWorker fr && isAbsent f.panes then some (.PaneGone, .Todo)
  else if isHD fr && f.answered && !f.waiting then some (.Answered, .Todo)
  else if isTW fr && isLive f.panes && f.waiting then some (.Waiting, .HumanDecision)
  else if isWorker fr && isLive f.panes && f.idle && oc then some (.StallCap, .HumanAction)
  else if isTodo fr && isLive f.panes && !(f.answered && !f.working) then some (.Live, .Worker)
  else none

/-- over_cap の Nat の上の値 -/
def overCapN (f : inbox_core.TaskFacts) (now cap : Nat) : Bool :=
  f.detected && f.seen && !f.cleared && decide (f.detected_at.val + cap * 60 ≤ now)

/-- 溢れの前提は stalled_refines と同じく、生成物が掛け算と足し算をする枝 (detected ∧ seen ∧ ¬cleared) だけに置く -/
theorem over_cap_eq (f : inbox_core.TaskFacts) (now cap : Std.U64)
    (hov : f.detected = true → f.seen = true → f.cleared = false →
      f.detected_at.val + cap.val * 60 ≤ U64.max) :
    inbox_core.over_cap f now cap = ok (overCapN f now.val cap.val) := by
  rcases f with ⟨pn, idle, wk, cf, acc, racc, rem, wt, ans, det, dat, seen, clr⟩
  simp only at hov ⊢
  cases det <;> cases seen <;> cases clr <;> simp [inbox_core.over_cap, overCapN]
  have h := hov rfl rfl rfl
  obtain ⟨i, hi, hiv⟩ := u64_mul_ok cap 60#u64 (by simp; omega)
  have hi60 : i.val = cap.val * 60 := by rw [hiv]; simp
  obtain ⟨j, hj, hjv⟩ := u64_add_ok dat i (by omega)
  simp [hi, hj]
  omega

theorem ite_ok {α} (c : Prop) [Decidable c] (a b : α) :
    (if c then (ok a : Result α) else ok b) = ok (if c then a else b) := by
  split <;> rfl

theorem next_lane_refines (fr : inbox_core.Lane) (f : inbox_core.TaskFacts) (now cap : Std.U64)
    (hov : f.detected = true → f.seen = true → f.cleared = false →
      f.detected_at.val + cap.val * 60 ≤ U64.max) :
    inbox_core.next_lane fr f now cap =
      ok ((laneSpec fr f (overCapN f now.val cap.val)).map fun p => ⟨p.1, p.2⟩) := by
  rcases f with ⟨pn, idle, wk, cf, acc, racc, rem, wt, ans, det, dat, seen, clr⟩
  -- 生成物は over_cap に { f with panes := p1, remaining := r } を渡し、r は場合分けの途中で式のまま残る。
  -- over_cap は panes と remaining を読まないので、そこを任意にした形で書き換える
  have hoc : ∀ pn' idle' wk' cf' acc' racc' rem' wt' ans',
      inbox_core.over_cap ⟨pn', idle', wk', cf', acc', racc', rem', wt', ans', det, dat, seen, clr⟩ now cap =
        ok (det && seen && !clr && decide (dat.val + cap.val * 60 ≤ now.val)) :=
    fun _ _ _ _ _ _ _ _ _ => over_cap_eq _ now cap hov
  unfold laneSpec
  cases fr <;> cases pn <;> cases rem <;>
    simp [inbox_core.next_lane, inbox_core.todo_or_worker, hoc, ite_ok, overCapN, isTW, isLive, isGone, isNothing, settleDest, isTodo, isWorker, isHD, isAbsent] <;>
    (clear hov hoc; split_ifs <;> simp_all)

/-! ## next_lane の性質 (仕様の上で) -/

/-- done / human_action からは移さない -/
theorem laneSpec_terminal (fr : inbox_core.Lane) (f : inbox_core.TaskFacts) (oc : Bool)
    (h : fr = .Done ∨ fr = .HumanAction) : laneSpec fr f oc = none := by
  rcases h with rfl | rfl <;> simp [laneSpec, isTW, isLive, isGone, isNothing, isTodo, isWorker, isHD, isAbsent]

/-- 行き先だけを見た 1 歩 step を、none で止まるか fuel が尽きるまで繰り返した lane の列 (始点を含む) -/
def laneTrace (step : inbox_core.Lane → Option inbox_core.Lane) : Nat → inbox_core.Lane → List inbox_core.Lane
  | 0, l => [l]
  | n + 1, l => match step l with
    | none => [l]
    | some d => l :: laneTrace step n d

/-! 列の性質は、1 歩ごとに真に減る順位 rk があれば fuel と事実の組に依らず従う。
収束を全組の場合分けで示すと 3 歩分の枝が掛け算で膨らむので、場合分けは 1 歩の laneSpec_rank_lt だけに閉じる -/

theorem laneTrace_rank_le (step : inbox_core.Lane → Option inbox_core.Lane) (rk : inbox_core.Lane → Nat)
    (hs : ∀ l d, step l = some d → rk d < rk l) :
    ∀ n l, ∀ x ∈ laneTrace step n l, rk x ≤ rk l
  | 0, l, x, hx => by simp [laneTrace] at hx; subst hx; exact Nat.le_refl _
  | n + 1, l, x, hx => by
    simp only [laneTrace] at hx
    split at hx
    · simp at hx; subst hx; exact Nat.le_refl _
    · rename_i d hd
      rcases List.mem_cons.1 hx with rfl | hx
      · exact Nat.le_refl _
      · have := laneTrace_rank_le step rk hs n d x hx
        have := hs l d hd
        omega

theorem laneTrace_nodup (step : inbox_core.Lane → Option inbox_core.Lane) (rk : inbox_core.Lane → Nat)
    (hs : ∀ l d, step l = some d → rk d < rk l) :
    ∀ n l, (laneTrace step n l).Nodup
  | 0, l => by simp [laneTrace]
  | n + 1, l => by
    simp only [laneTrace]
    split
    · simp
    · rename_i d hd
      refine List.nodup_cons.2 ⟨fun hm => ?_, laneTrace_nodup step rk hs n d⟩
      have := laneTrace_rank_le step rk hs n d l hm
      have := hs l d hd
      omega

theorem laneTrace_stops (step : inbox_core.Lane → Option inbox_core.Lane) (rk : inbox_core.Lane → Nat)
    (hs : ∀ l d, step l = some d → rk d < rk l) :
    ∀ n l, rk l ≤ n → ∃ x, (laneTrace step n l).getLast? = some x ∧ step x = none
  | 0, l, h => by
    refine ⟨l, by simp [laneTrace], ?_⟩
    cases hl : step l with
    | none => rfl
    | some d => have := hs l d hl; omega
  | n + 1, l, h => by
    simp only [laneTrace]
    split
    · rename_i hd; exact ⟨l, by simp, hd⟩
    · rename_i d hd
      obtain ⟨x, hx, hn⟩ := laneTrace_stops step rk hs n d (by have := hs l d hd; omega)
      exact ⟨x, by simp [List.getLast?_cons, hx], hn⟩

/-- 判断待ちの human_decision は移さないので 0。pane が生きていれば todo → worker、無ければ worker → todo の向きにだけ動く -/
def laneRank (f : inbox_core.TaskFacts) : inbox_core.Lane → Nat
  | .Done | .HumanAction => 0
  | .HumanDecision => if f.waiting then 0 else 3
  | .Todo => if isLive f.panes then 2 else 1
  | .Worker => if isLive f.panes then 1 else 2

theorem laneRank_le (f : inbox_core.TaskFacts) (l : inbox_core.Lane) : laneRank f l ≤ 3 := by
  cases l <;> simp only [laneRank] <;> (try split) <;> omega

theorem laneSpec_rank_lt (fr : inbox_core.Lane) (f : inbox_core.TaskFacts) (oc : Bool)
    (r : inbox_core.Rule) (d : inbox_core.Lane) (h : laneSpec fr f oc = some (r, d)) :
    laneRank f d < laneRank f fr := by
  rcases f with ⟨pn, idle, wk, cf, acc, racc, rem, wt, ans, det, dat, seen, clr⟩
  unfold laneSpec at h
  cases fr <;> cases pn <;> cases rem <;> simp [isTW, isLive, isGone, isNothing, settleDest, isTodo, isWorker, isHD, isAbsent] at h <;> (try split_ifs at h) <;>
    (try simp at h) <;> (try casesm* _ ∧ _) <;> subst_vars <;> simp_all [laneRank, isLive]

theorem laneSpec_dest_ne (fr : inbox_core.Lane) (f : inbox_core.TaskFacts) (oc : Bool)
    (r : inbox_core.Rule) (d : inbox_core.Lane) (h : laneSpec fr f oc = some (r, d)) : d ≠ fr := by
  rintro rfl
  exact Nat.lt_irrefl _ (laneSpec_rank_lt d f oc r d h)

/-- tests/lane.rs の全数の試験と同じ主張。事実を固定して繰り返すと、3 回以内に移さない lane に着き、
通った lane に戻らない。fuel 3 で切った列の末尾で step が none なので、3 回で止まっている -/
theorem laneSpec_converges (fr : inbox_core.Lane) (f : inbox_core.TaskFacts) (oc : Bool) :
    let step := fun l => (laneSpec l f oc).map Prod.snd
    (∃ l, (laneTrace step 3 fr).getLast? = some l ∧ step l = none) ∧ (laneTrace step 3 fr).Nodup := by
  intro step
  have hs : ∀ l d, step l = some d → laneRank f d < laneRank f l := by
    intro l d h
    simp only [step, Option.map_eq_some_iff] at h
    obtain ⟨⟨r, d'⟩, h1, rfl⟩ := h
    exact laneSpec_rank_lt l f oc r d' h1
  exact ⟨laneTrace_stops step _ hs 3 fr (laneRank_le f fr), laneTrace_nodup step _ hs 3 fr⟩

/-- end_marker.rs が報告を落として呼び直したとき、next_lane の仕様は done を返さない。 -/
theorem laneSpec_done_requires_report (fr : inbox_core.Lane) (f : inbox_core.TaskFacts) (oc : Bool)
    (r : inbox_core.Rule) (h : laneSpec fr f oc = some (r, .Done)) :
    f.accepted = true ∨ f.req_accepted = true := by
  cases ha : f.accepted <;> cases hr : f.req_accepted <;> simp_all
  simp [laneSpec, ha, hr] at h
  split_ifs at h <;> simp_all

/-! ## next_lane の性質 (生成物の上で) -/

theorem next_lane_terminal (fr : inbox_core.Lane) (f : inbox_core.TaskFacts) (now cap : Std.U64)
    (h : fr = .Done ∨ fr = .HumanAction) : inbox_core.next_lane fr f now cap = ok none := by
  rcases f with ⟨pn, idle, wk, cf, acc, racc, rem, wt, ans, det, dat, seen, clr⟩
  rcases h with rfl | rfl <;> cases pn <;> cases rem <;> simp [inbox_core.next_lane, inbox_core.todo_or_worker]

theorem next_lane_dest_ne (fr : inbox_core.Lane) (f : inbox_core.TaskFacts) (now cap : Std.U64)
    (hov : f.detected = true → f.seen = true → f.cleared = false →
      f.detected_at.val + cap.val * 60 ≤ U64.max)
    (m : inbox_core.Move) (h : inbox_core.next_lane fr f now cap = ok (some m)) : m.dest ≠ fr := by
  rw [next_lane_refines fr f now cap hov, Result.ok.injEq, Option.map_eq_some_iff] at h
  obtain ⟨⟨r, d⟩, h1, rfl⟩ := h
  exact laneSpec_dest_ne fr f _ r d h1

theorem next_lane_done_requires_report (fr : inbox_core.Lane) (f : inbox_core.TaskFacts) (now cap : Std.U64)
    (hov : f.detected = true → f.seen = true → f.cleared = false →
      f.detected_at.val + cap.val * 60 ≤ U64.max)
    (m : inbox_core.Move) (h : inbox_core.next_lane fr f now cap = ok (some m))
    (hd : m.dest = .Done) : f.accepted = true ∨ f.req_accepted = true := by
  rw [next_lane_refines fr f now cap hov, Result.ok.injEq, Option.map_eq_some_iff] at h
  obtain ⟨⟨r, d⟩, h, rfl⟩ := h
  change d = .Done at hd
  subst d
  exact laneSpec_done_requires_report fr f _ r h

/-- Result は帰納型でなく match で ok を取り出せないので、生成物の 1 歩 step は next_lane との関係で与える -/
theorem next_lane_converges (fr : inbox_core.Lane) (f : inbox_core.TaskFacts) (now cap : Std.U64)
    (hov : f.detected = true → f.seen = true → f.cleared = false →
      f.detected_at.val + cap.val * 60 ≤ U64.max)
    (step : inbox_core.Lane → Option inbox_core.Lane)
    (hstep : ∀ l d, step l = some d ↔ ∃ m, inbox_core.next_lane l f now cap = ok (some m) ∧ m.dest = d) :
    (∃ l, (laneTrace step 3 fr).getLast? = some l ∧ step l = none) ∧ (laneTrace step 3 fr).Nodup := by
  have he : step = fun l => (laneSpec l f (overCapN f now.val cap.val)).map Prod.snd := by
    funext l
    have hs := fun d => hstep l d
    simp only [next_lane_refines l f now cap hov, Result.ok.injEq] at hs
    cases h : laneSpec l f (overCapN f now.val cap.val) with
    | none =>
      cases h' : step l with
      | none => rfl
      | some d => simp [h, h'] at hs
    | some p =>
      have := (hs p.2).2 ⟨⟨p.1, p.2⟩, by simp [h], rfl⟩
      simp [this]
  rw [he]
  exact laneSpec_converges fr f _

/-- next_lane_converges の前提 hstep を満たす step は在る (前提が空でない) -/
theorem next_lane_step_exists (f : inbox_core.TaskFacts) (now cap : Std.U64)
    (hov : f.detected = true → f.seen = true → f.cleared = false →
      f.detected_at.val + cap.val * 60 ≤ U64.max) :
    ∃ step : inbox_core.Lane → Option inbox_core.Lane, ∀ l d,
      step l = some d ↔ ∃ m, inbox_core.next_lane l f now cap = ok (some m) ∧ m.dest = d := by
  refine ⟨fun l => (laneSpec l f (overCapN f now.val cap.val)).map Prod.snd, fun l d => ?_⟩
  simp only [next_lane_refines l f now cap hov, Result.ok.injEq, Option.map_eq_some_iff]
  constructor
  · rintro ⟨p, hp, rfl⟩; exact ⟨⟨p.1, p.2⟩, ⟨p, hp, rfl⟩, rfl⟩
  · rintro ⟨m, ⟨p, hp, rfl⟩, rfl⟩; exact ⟨p, hp, rfl⟩

end InboxProofs

#print axioms InboxProofs.resend_step_total
#print axioms InboxProofs.resend_enter_iff
#print axioms InboxProofs.resend_nudge_iff
#print axioms InboxProofs.resend_exhausted_iff
#print axioms InboxProofs.resend_overdue_no_nudge
#print axioms InboxProofs.resend_tried_no_enter
#print axioms InboxProofs.over_cap_eq
#print axioms InboxProofs.ite_ok
#print axioms InboxProofs.next_lane_refines
#print axioms InboxProofs.laneSpec_terminal
#print axioms InboxProofs.laneTrace_rank_le
#print axioms InboxProofs.laneTrace_nodup
#print axioms InboxProofs.laneTrace_stops
#print axioms InboxProofs.laneRank_le
#print axioms InboxProofs.laneSpec_rank_lt
#print axioms InboxProofs.laneSpec_dest_ne
#print axioms InboxProofs.laneSpec_converges
#print axioms InboxProofs.next_lane_terminal
#print axioms InboxProofs.next_lane_dest_ne
#print axioms InboxProofs.next_lane_converges
#print axioms InboxProofs.next_lane_step_exists
#print axioms InboxProofs.laneSpec_done_requires_report
#print axioms InboxProofs.next_lane_done_requires_report
