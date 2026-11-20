import Aeneas
import InboxCore.InboxCore
import Handoff.Post
open Aeneas Std Result

/-! Aeneas が生成した inbox_core.step が Handoff の model の Step を精密化することの証明。
inbox_core と Handoff は Msg / Tag / Res の名前が重なるので、どちらも open しない。 -/
namespace InboxProofs

/-- Info / Conflict / Other は model の Tag に無い。ここで返す値は意味を持たない埋め草で、
model の主張に関わる定理はどれも InModel を仮定に取って埋め草の像を使わない -/
def tagM : inbox_core.Tag → Handoff.Tag
  | .Escalation => .escalation | .Request => .request | .Alert => .alert
  | .Completion => .completion | .Progress => .progress | .Reply => .reply
  | .Info => .completion | .Conflict => .completion | .Other => .alert

/-- model の Tag に像を持つタグ -/
def InModel : inbox_core.Tag → Prop
  | .Info | .Conflict | .Other => False
  | _ => True

def placeM : inbox_core.Place → Handoff.Box
  | .New => .new | .Held => .held | .Cur => .cur

def resM : inbox_core.Res → Handoff.Res
  | .None => .none | .Replied => .replied | .NoReply => .noReply | .SenderGone => .senderGone

/-- surfaced / skipped は Rust 側に無いので、model の msg から引き継ぐ -/
def toModel (m : inbox_core.Msg) (sf sk : Bool) : Handoff.Msg :=
  { tag := tagM m.tag, sender := m.sender.val, box := placeM m.place, res := resM m.res,
    surfaced := sf, skipped := sk }

/-- 外殻の観測が model の状態と整合する (README の A5 / A6 にあたる仮定)。
片方向だけを置く: Alive / Unreadable は Refused にしかならないので仮定が要らない -/
structure Consistent (s : Handoff.State) (m : inbox_core.Msg) (o : inbox_core.Obs) : Prop where
  gone : o.sender = .Gone → s.senderAlive m.sender.val = false
  sent : o.sent = true → s.senderAlive m.sender.val = true

/-! ## 1. 全域性 -/

theorem step_total (m : inbox_core.Msg) (op : inbox_core.Op) (o : inbox_core.Obs) :
    ∃ r, inbox_core.step m op o = ok r := by
  rcases m with ⟨t, snd, p, r⟩
  rcases o with ⟨os, sent, hv⟩
  cases op <;> cases p <;> cases t <;> cases os <;> cases sent <;> cases hv <;>
    simp [inbox_core.step, inbox_core.to_cur, inbox_core.needs_reply]

/-! ## 2. refinement (Rust ⊆ model)

Unchanged / Refused は model の状態を s のまま置く (stutter) ので、ここで述べるのは Moved だけ -/

set_option linter.unusedSimpArgs false in
set_option linter.unusedTactic false in
/-- c.replyGate を仮定に取らない: Rust は常に gate 付きの判定をするので、どの Config の Step にも収まる。
ヘッダの正否も仮定に取らない: ヘッダ不正で Moved を返す分岐 (--no-reply で escalation / request を閉じる) を外したため -/
theorem step_refines (c : Handoff.Config) (w : Handoff.World)
    (s : Handoff.State) (mid : Handoff.MId) (m : inbox_core.Msg) (sf sk : Bool)
    (op : inbox_core.Op) (o : inbox_core.Obs) (m' : inbox_core.Msg)
    (ht : InModel m.tag)
    (hs : s.msgs mid = some (toModel m sf sk)) (ho : Consistent s m o)
    (h : inbox_core.step m op o = ok (.Moved m')) :
    ∃ s', Handoff.Step c w s s' ∧ s'.msgs mid = some (toModel m' sf sk) := by
  rcases m with ⟨t, snd, p, r⟩
  rcases o with ⟨os, sent, hv⟩
  cases t <;> simp only [InModel] at ht <;>
  cases op <;> cases p <;> cases os <;> cases sent <;> cases hv <;>
    simp [inbox_core.step, inbox_core.to_cur, inbox_core.needs_reply] at h <;> subst h
  all_goals first
    | (refine ⟨_, Handoff.Step.ack s mid _ hs ?_ ?_, ?_⟩ <;> (simp [toModel, placeM, tagM, resM, Handoff.Tag.needsReply, Handoff.toCur]; done))
    | (refine ⟨_, Handoff.Step.ackNoReply s mid _ hs ?_ ?_, ?_⟩ <;> (simp [toModel, placeM, tagM, resM, Handoff.Tag.needsReply, Handoff.toCur]; done))
    | (refine ⟨_, Handoff.Step.ackSenderGone s mid _ hs ?_ (ho.gone rfl), ?_⟩ <;> (simp [toModel, placeM, tagM, resM, Handoff.Tag.needsReply, Handoff.toCur]; done))
    | (refine ⟨_, Handoff.Step.reply s mid _ hs ?_ (ho.sent rfl), ?_⟩ <;> (simp [toModel, placeM, tagM, resM, Handoff.Tag.needsReply, Handoff.toCur]; done))
    | (refine ⟨_, Handoff.Step.hold s mid _ hs ?_, ?_⟩ <;> (simp [toModel, placeM, tagM, resM, Handoff.Tag.needsReply, Handoff.toCur]; done))

/-! ## 3. 系: 到達可能性と E4 -/

theorem step_moved_e4 (c : Handoff.Config) (hc : c.replyGate = true) (w : Handoff.World)
    (s : Handoff.State) (hr : Handoff.Reach c w s) (mid : Handoff.MId) (m : inbox_core.Msg)
    (sf sk : Bool) (op : inbox_core.Op) (o : inbox_core.Obs) (m' : inbox_core.Msg)
    (ht : InModel m.tag)
    (hs : s.msgs mid = some (toModel m sf sk)) (ho : Consistent s m o)
    (h : inbox_core.step m op o = ok (.Moved m')) :
    ∃ s', Handoff.Reach c w s' ∧ s'.msgs mid = some (toModel m' sf sk) ∧
      ((toModel m' sf sk).box = .cur → (toModel m' sf sk).tag.needsReply = true →
        Handoff.Resolved s' mid (toModel m' sf sk)) := by
  obtain ⟨s', hst, hm⟩ := step_refines c w s mid m sf sk op o m' ht hs ho h
  have hr' := Handoff.Reach.step hr hst
  exact ⟨s', hr', hm, Handoff.e4_of c hc w s' hr' mid _ hm⟩

/-! ## 4. 空虚でないことの証拠 -/

/-- escalation は送信成功の reply で cur (res=replied) へ移る -/
theorem escalation_reply_moves :
    inbox_core.step ⟨.Escalation, 2#u32, .New, .None⟩ .Reply ⟨.Alive, true, true⟩ =
      ok (.Moved ⟨.Escalation, 2#u32, .Cur, .Replied⟩) := by rfl

/-- 同じ msg を理由無しの ack で閉じようとすると断られる -/
theorem escalation_ack_refused :
    inbox_core.step ⟨.Escalation, 2#u32, .New, .None⟩ .Ack ⟨.Alive, false, true⟩ =
      ok (.Refused .NeedsReply) := by simp [inbox_core.step, inbox_core.needs_reply]

/-- reply の結果を model に反映した状態は到達可能で、その msg は cur にあり Resolved -/
theorem escalation_reply_reach :
    ∃ s', Handoff.Reach Handoff.Config.post Handoff.wOk s' ∧
      ∃ x, s'.msgs 0 = some x ∧ x.box = .cur ∧ x.tag = .escalation ∧ Handoff.Resolved s' 0 x := by
  let m : inbox_core.Msg := ⟨.Escalation, 2#u32, .New, .None⟩
  let s1 : Handoff.State :=
    { Handoff.init with msgs := Handoff.upd Handoff.init.msgs 0 (some (Handoff.newMsg .escalation 2)) }
  have r1 : Handoff.Reach Handoff.Config.post Handoff.wOk s1 :=
    Handoff.Reach.step Handoff.Reach.init (Handoff.Step.send _ 0 .escalation 2 rfl)
  have hs : s1.msgs 0 = some (toModel m false false) := by
    simp [s1, m, toModel, Handoff.newMsg, tagM, placeM, resM]
  obtain ⟨s', hr', hm, he⟩ := step_moved_e4 Handoff.Config.post rfl Handoff.wOk s1 r1 0 m false false
    .Reply ⟨.Alive, true, true⟩ _ trivial hs ⟨(fun h => by cases h), (fun _ => rfl)⟩ escalation_reply_moves
  exact ⟨s', hr', _, hm, rfl, rfl, he rfl rfl⟩

/-! ## 5. 逆側 (model ⊆ Rust)

model の遷移の guard を満たす msg には、対応する入力で Rust が Moved を返し、その像が model の遷移後の msg に一致する -/

theorem ack_complete (m : inbox_core.Msg) (sf sk : Bool) (o : inbox_core.Obs) (hm : InModel m.tag)
    (hb : (toModel m sf sk).box ≠ .cur) (hn : (toModel m sf sk).tag.needsReply = false) :
    ∃ m', inbox_core.step m .Ack o = ok (.Moved m') ∧
      toModel m' sf sk = Handoff.toCur (toModel m sf sk) .none := by
  rcases m with ⟨t, snd, p, r⟩
  cases p <;> cases t <;> simp only [InModel] at hm <;>
    simp_all [toModel, placeM, tagM, Handoff.Tag.needsReply, inbox_core.step, inbox_core.to_cur,
      inbox_core.needs_reply, Handoff.toCur, resM]

theorem ackNoReply_complete (m : inbox_core.Msg) (sf sk : Bool) (o : inbox_core.Obs) (hm : InModel m.tag)
    (hb : (toModel m sf sk).box ≠ .cur)
    (ht : (toModel m sf sk).tag ≠ .escalation ∧ (toModel m sf sk).tag ≠ .request) :
    ∃ m', inbox_core.step m .AckNoReply o = ok (.Moved m') ∧
      toModel m' sf sk = Handoff.toCur (toModel m sf sk) .noReply := by
  rcases m with ⟨t, snd, p, r⟩
  cases p <;> cases t <;> simp only [InModel] at hm <;>
    simp_all [toModel, placeM, tagM, inbox_core.step, inbox_core.to_cur, Handoff.toCur, resM]

theorem ackSenderGone_complete (m : inbox_core.Msg) (sf sk sent : Bool)
    (hb : (toModel m sf sk).box ≠ .cur) :
    ∃ m', inbox_core.step m .AckSenderGone ⟨.Gone, sent, true⟩ = ok (.Moved m') ∧
      toModel m' sf sk = Handoff.toCur (toModel m sf sk) .senderGone := by
  rcases m with ⟨t, snd, p, r⟩
  cases p <;> simp_all [toModel, placeM, inbox_core.step, inbox_core.to_cur, Handoff.toCur, resM]

theorem reply_complete (m : inbox_core.Msg) (sf sk : Bool) (os : inbox_core.Sender)
    (hb : (toModel m sf sk).box ≠ .cur) :
    ∃ m', inbox_core.step m .Reply ⟨os, true, true⟩ = ok (.Moved m') ∧
      toModel m' sf sk = Handoff.toCur (toModel m sf sk) .replied := by
  rcases m with ⟨t, snd, p, r⟩
  cases p <;> simp_all [toModel, placeM, inbox_core.step, inbox_core.to_cur, Handoff.toCur, resM]

theorem hold_complete (m : inbox_core.Msg) (sf sk : Bool) (o : inbox_core.Obs)
    (hb : (toModel m sf sk).box = .new) :
    ∃ m', inbox_core.step m .Hold o = ok (.Moved m') ∧
      toModel m' sf sk = { toModel m sf sk with box := .held } := by
  rcases m with ⟨t, snd, p, r⟩
  cases p <;> simp_all [toModel, placeM, inbox_core.step]

/-! ## ヘッダ不正の分岐を外したこと

w8R の bad_header_breaks_e4 は「ヘッダ不正の escalation / request を --no-reply で閉じると E4 が崩れる」を示した。
その分岐を外したので、言明を「ヘッダの正否に依らず断る」に置き換えた -/

theorem no_reply_refused_any_header (m : inbox_core.Msg) (o : inbox_core.Obs)
    (ht : m.tag = .Escalation ∨ m.tag = .Request) (hp : m.place ≠ .Cur) :
    inbox_core.step m .AckNoReply o = ok (.Refused .NeedsReply) := by
  rcases m with ⟨t, snd, p, r⟩
  simp only at ht hp
  rcases ht with rfl | rfl <;> cases p <;> simp_all [inbox_core.step]

/-! ## model に無いタグ (InModel の外)

info / conflict は返信不要として ack で閉じられ、語彙外 (Other) は ack を断られるが --no-reply で閉じられる。
どれも model の Tag に像が無いので、E4 はこれらについて何も言わない (step_refines は InModel を仮定に取る) -/

theorem other_no_reply_moves (snd : Std.U32) (p : inbox_core.Place) (o : inbox_core.Obs) (hp : p ≠ .Cur) :
    inbox_core.step ⟨.Other, snd, p, .None⟩ .AckNoReply o = ok (.Moved ⟨.Other, snd, .Cur, .NoReply⟩) := by
  cases p <;> simp_all [inbox_core.step, inbox_core.to_cur]

end InboxProofs

#print axioms InboxProofs.step_total
#print axioms InboxProofs.step_refines
#print axioms InboxProofs.step_moved_e4
#print axioms InboxProofs.escalation_reply_moves
#print axioms InboxProofs.escalation_ack_refused
#print axioms InboxProofs.escalation_reply_reach
#print axioms InboxProofs.ack_complete
#print axioms InboxProofs.ackNoReply_complete
#print axioms InboxProofs.ackSenderGone_complete
#print axioms InboxProofs.reply_complete
#print axioms InboxProofs.hold_complete
#print axioms InboxProofs.no_reply_refused_any_header
#print axioms InboxProofs.other_no_reply_moves
