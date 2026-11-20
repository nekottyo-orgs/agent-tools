import Handoff.Model
/-! 修正後の形 (Config.post) で取りこぼしの性質を証明する -/
namespace Handoff

theorem upd_some_pres {α β : Type} [DecidableEq α] {P : α → β → Prop} (f : α → Option β) (a : α) (b : β)
    (ih : ∀ i x, f i = some x → P i x) (h : P a b) : ∀ i x, upd f a (some b) i = some x → P i x := by
  intro i x hx
  rw [upd_apply] at hx
  split at hx
  · rename_i hi; subst hi; cases hx; exact h
  · exact ih i x hx

theorem map_pres {P : MId → Msg → Prop} (g : Msg → Msg) (ms : MId → Option Msg)
    (ih : ∀ i x, ms i = some x → P i x) (hg : ∀ i x, P i x → P i (g x)) :
    ∀ i y, mapMsgs g ms i = some y → P i y := by
  intro i y hy
  unfold mapMsgs at hy
  cases h : ms i with
  | none => simp [h] at hy
  | some x => simp [h] at hy; subst hy; exact hg i x (ih i x h)

/-! ### 接頭辞 (pre-add を通る) -/

/-- 窓口と Cue worker の pane が pre-add に識別され (hreg)、HTTP で登録しない (hhttp) なら、
窓口と Cue worker からの登録はどの経路でも自 run の接頭辞を持つ -/
theorem post_prefix (w : World) (hreg : ∀ p, w.fd p = true → w.reg p = true)
    (hhttp : w.httpFromFd = false) : PrefixProp Config.post w := by
  intro s hr
  induction hr with
  | init => intro d x h; simp [init] at h
  | @step s0 s1 hr hs ih =>
    cases hs with
    | fault h => cases h <;> exact ih
    | rearm h => cases h <;> exact ih
    | human h => cases h; simp [Config.post]; exact ih
    | add p d k r _ hal =>
      dsimp only
      refine upd_some_pres _ _ _ ih ?_
      intro hfd
      simp [newDec] at hfd ⊢
      cases r with
      | script => exact hal
      | rawPlain => exact (hal.2 rfl).2 (hreg p hfd)
      | rawQuoted => exact (hal rfl).2 (hreg p hfd)
      | http => have := hal hfd; simp [hhttp] at this
    | answer d x hx _ => dsimp only; exact upd_some_pres _ _ _ ih (ih d x hx)
    | claimStdout d x t h1 => simp [Config.post] at h1
    | scriptClaim d x t h1 => simp [Config.post] at h1
    | txCommit d x t _ _ hx _ => dsimp only; exact upd_some_pres _ _ _ ih (ih d x hx)
    | complete d x t hx _ _ => dsimp only; exact upd_some_pres _ _ _ ih (ih d x hx)
    | expire d x t hx _ => dsimp only; exact upd_some_pres _ _ _ ih (ih d x hx)
    | _ => exact ih

/-! ### E2 -/

/-- 接頭辞の性質と検知があれば、回答済みで未 claim の判断は次の 1 歩で検知されうる -/
theorem post_e2 (w : World) (hreg : ∀ p, w.fd p = true → w.reg p = true)
    (hhttp : w.httpFromFd = false) : E2Prop Config.post w := by
  intro s hr d x hx hfd ha hc hd
  have hp := post_prefix w hreg hhttp s hr d x hx hfd
  exact ⟨_, Step.detect s d x rfl hx hp ha hc hd, Or.inl (by simp)⟩

/-! ### E7 -/

def Inv7 (s : State) : Prop :=
  (∀ d x, s.cue d = some x → ∀ t, x.claim = some t → s.file t = some d) ∧
  (∀ d t, s.pending = some (d, t) → s.file t = some d ∧ ∃ x, s.cue d = some x ∧ x.claim = none) ∧
  (∀ d, s.mem d = none)

theorem post_inv7 (w : World) : ∀ s, Reach Config.post w s → Inv7 s := by
  intro s hr
  induction hr with
  | init => refine ⟨?_, ?_, ?_⟩ <;> intros <;> simp_all [init]
  | @step s0 s1 hr hs ih =>
    obtain ⟨h1, h2, h3⟩ := ih
    cases hs with
    | fault h =>
      cases h with
      | crash => exact ⟨h1, by simp, by simp⟩
      | outputCut => exact ⟨h1, h2, by simp⟩
      | _ => exact ⟨h1, h2, h3⟩
    | rearm h => cases h <;> exact ⟨h1, h2, h3⟩
    | human h => cases h; simp [Config.post]; exact ⟨h1, h2, h3⟩
    | add p d k r hnone _ =>
      refine ⟨?_, ?_, h3⟩
      · dsimp only; refine upd_some_pres _ _ _ h1 ?_; simp [newDec]
      · intro d' t hp
        obtain ⟨hf, y, hy, hyc⟩ := h2 d' t hp
        refine ⟨hf, y, ?_, hyc⟩
        have : d' ≠ d := by intro e; subst e; simp [hnone] at hy
        simp [upd_apply, this, hy]
    | answer d x hx _ =>
      refine ⟨?_, ?_, h3⟩
      · dsimp only; exact upd_some_pres _ _ _ h1 (h1 d x hx)
      · intro d' t hp
        obtain ⟨hf, y, hy, hyc⟩ := h2 d' t hp
        refine ⟨hf, ?_⟩
        by_cases e : d' = d
        · subst e; rw [hx] at hy; cases hy; exact ⟨{ x with answered := true }, by simp, hyc⟩
        · exact ⟨y, by simp [upd_apply, e, hy], hyc⟩
    | claimStdout d x t h0 => simp [Config.post] at h0
    | save d t h0 => simp [Config.post] at h0
    | scriptClaim d x t h0 => simp [Config.post] at h0
    | scriptFlush d t h0 => simp [Config.post] at h0
    | txPublish d x t _ hcl _ hpn hft =>
      obtain ⟨hx, _, _, hxc, _, _⟩ := hcl
      refine ⟨?_, ?_, h3⟩
      · intro d' y hy t' hyt
        have hf := h1 d' y hy t' hyt
        have : t' ≠ t := by intro e; subst e; simp [hft] at hf
        simp [upd_apply, this, hf]
      · intro d' t' hp
        simp at hp; obtain ⟨rfl, rfl⟩ := hp
        exact ⟨by simp, x, hx, hxc⟩
    | txCommit d x t _ hpn hx hxc =>
      obtain ⟨hf, _⟩ := h2 d t hpn
      refine ⟨?_, by simp, h3⟩
      dsimp only
      refine upd_some_pres _ _ _ h1 ?_
      intro t' ht'; simp at ht'; subst ht'; exact hf
    | complete d x t hx hxt hor =>
      have hft : s0.file t = some d := by
        rcases hor with h | h
        · simp [h3 d] at h
        · exact h
      refine ⟨?_, ?_, h3⟩
      · intro d' y hy t' hyt
        by_cases e : d' = d
        · subst e; simp at hy; subst hy; simp at hyt
        · simp [upd_apply, e] at hy
          have hf := h1 d' y hy t' hyt
          have : t' ≠ t := by intro e2; subst e2; rw [hft] at hf; cases hf; exact e rfl
          simp [upd_apply, this, hf]
      · intro d' t' hp
        obtain ⟨hf, y, hy, hyc⟩ := h2 d' t' hp
        have e : d' ≠ d := by intro e; subst e; rw [hx] at hy; cases hy; simp [hxt] at hyc
        have e2 : t' ≠ t := by intro e2; subst e2; rw [hft] at hf; cases hf; exact e rfl
        exact ⟨by simp [upd_apply, e2, hf], y, by simp [upd_apply, e, hy], hyc⟩
    | expire d x t hx hxt =>
      refine ⟨?_, ?_, h3⟩
      · dsimp only; refine upd_some_pres _ _ _ h1 ?_; simp
      · intro d' t' hp
        obtain ⟨hf, y, hy, hyc⟩ := h2 d' t' hp
        have e : d' ≠ d := by intro e; subst e; rw [hx] at hy; cases hy; simp [hxt] at hyc
        exact ⟨hf, y, by simp [upd_apply, e, hy], hyc⟩
    | _ => exact ⟨h1, h2, h3⟩

/-- claim した token は、process の消失・出力の切断・保存先の停止のどれが起きてもファイルに残る -/
theorem post_e7 (w : World) : E7Prop Config.post w := by
  intro s hr d x t hx ht
  exact Or.inl ((post_inv7 w s hr).1 d x hx t ht)

/-! ### E4 / E6 -/

def P4 (R : MId → Bool) (m : MId) (x : Msg) : Prop :=
  x.box = .cur → x.tag.needsReply = true →
    (x.res = .replied ∧ R m = true) ∨ (x.tag = .alert ∧ x.res = .noReply) ∨ x.res = .senderGone

theorem keep4 {R : MId → Bool} {ms : MId → Option Msg} (g : Msg → Msg)
    (ih : ∀ i x, ms i = some x → P4 R i x)
    (hg : ∀ x, (g x).box = x.box ∧ (g x).res = x.res ∧ (g x).tag = x.tag) :
    ∀ i y, mapMsgs g ms i = some y → P4 R i y :=
  map_pres g ms ih (fun i x hp hb hn => by
    obtain ⟨e1, e2, e3⟩ := hg x
    rw [e1] at hb; rw [e3] at hn; have := hp hb hn; rw [e2, e3]; exact this)

/-- reply の対 (ADR-316) だけに依る。移行期 (stage0) でも成り立つ -/
theorem e4_of (c : Config) (hc : c.replyGate = true) (w : World) : E4Prop c w := by
  suffices H : ∀ s, Reach c w s → ∀ m x, s.msgs m = some x → P4 s.replied m x by
    intro s hr m x hx hb hn; exact H s hr m x hx hb hn
  intro s hr
  induction hr with
  | init => intro m x h; simp [init] at h
  | @step s0 s1 hr hs ih =>
    cases hs with
    | fault h => cases h <;> exact ih
    | rearm h =>
      cases h with
      | reveal => exact keep4 _ ih (fun x => by split <;> simp)
      | arm b _ =>
        cases b
        · exact keep4 _ ih (fun x => by simp)
        · exact keep4 _ ih (fun x => by split <;> simp)
    | human h =>
      cases h; split
      · exact keep4 _ ih (fun x => by split <;> simp)
      · exact ih
    | fire _ => exact keep4 _ ih (fun x => by split <;> simp)
    | send m t p _ =>
      dsimp only; refine upd_some_pres _ _ _ ih ?_; intro hb; simp [newMsg] at hb
    | ack m x hx _ hg =>
      dsimp only; refine upd_some_pres _ _ _ ih ?_
      intro _ hn; simp [toCur] at hn; simp [hg hc] at hn
    | ackNoReply m x hx _ hg =>
      dsimp only; refine upd_some_pres _ _ _ ih ?_
      intro _ hn
      obtain ⟨a, b⟩ := hg hc
      simp [toCur] at hn ⊢
      cases ht : x.tag <;> simp_all [Tag.needsReply]
    | ackSenderGone m x hx _ _ =>
      dsimp only; refine upd_some_pres _ _ _ ih ?_; intro _ _; simp [toCur]
    | reply m x hx _ _ =>
      dsimp only
      refine upd_some_pres (P := P4 (upd s0.replied m true)) _ _ _ (fun i y hi => ?_) (by intro _ _; simp [toCur])
      intro hb hn
      rcases ih i y hi hb hn with ⟨h1, h2⟩ | h | h
      · left; refine ⟨h1, ?_⟩; rw [upd_apply]; split <;> simp [h2]
      · exact Or.inr (Or.inl h)
      · exact Or.inr (Or.inr h)
    | hold m x hx hb =>
      dsimp only; refine upd_some_pres _ _ _ ih ?_; intro hb'; simp at hb'
    | _ => exact ih

theorem post_e4 (w : World) : E4Prop Config.post w := e4_of _ rfl w
theorem stage0_e4 (w : World) : E4Prop Config.stage0 w := e4_of _ rfl w

/-! ### E5 -/

theorem showNew_surfaces (ms : MId → Option Msg) (m : MId) (x : Msg) (hx : ms m = some x)
    (hb : x.box = .new) : ∃ y, showNew ms m = some y ∧ y.surfaced = true :=
  ⟨{ x with surfaced := true }, by simp [showNew, mapMsgs, hx, hb], rfl⟩

/-- new/ に残る報告は、次の張り直しでも人の発言でも窓口に出る。Reach を使わない構成上の定理で、
中身は「張り直しと人の発言が new/ の残りを即時に全件出す」(README の前提 A7) に依る -/
theorem e5_of (c : Config) (h1 : c.rearmSurface = true) (h2 : c.humanSurface = true) (w : World) :
    E5Prop c w := by
  intro s _ m x hx hb
  refine ⟨fun s' h => ?_, fun s' h => ?_⟩
  · cases h with
    | reveal => exact showNew_surfaces _ m x hx hb
    | arm b hn => exact absurd ⟨h1, ⟨m, x, hx, hb⟩⟩ hn
  · cases h; simp only [h2, ite_true]; exact showNew_surfaces _ m x hx hb

theorem post_e5 (w : World) : E5Prop Config.post w := e5_of _ rfl rfl w
/-- 移行期でもモデルの上では成り立つ。現実の PR 318 は 10 分の閾値と「同じ組は 1 回だけ」を通すので、
このモデルは移行期の E5 を現実より強く表している (README の G5) -/
theorem stage0_e5 (w : World) : E5Prop Config.stage0 w := e5_of _ rfl rfl w

/-- 故障は inbox を変えない (受信待ちが止まっても報告は new/ に残る)。どの設定でも成り立つ -/
theorem fault_keeps_msgs (s s' : State) (h : Fault s s') : s'.msgs = s.msgs := by
  cases h <;> rfl

/-! ### 修正後のモデルが進めること (安全性が「何もできない」ことで成り立っていない証人) -/

def wOk : World := { fd := fun p => p == 1, reg := fun p => p == 1, httpFromFd := false }

/-- add → answer → claim (ファイル公開 → commit) → complete が修正後でも通る -/
theorem post_progress_claim : ∃ s, Reach Config.post wOk s ∧ ∃ x, s.cue 0 = some x ∧ x.done = true := by
  let d0 := newDec wOk 1 .own
  let s1 : State := { init with cue := upd init.cue 0 (some d0) }
  let d1 : Dec := { d0 with answered := true }
  let s2 : State := { s1 with cue := upd s1.cue 0 (some d1) }
  have r2 : Reach Config.post wOk s2 :=
    Reach.step (Reach.step Reach.init (Step.add init 1 0 .own .rawQuoted rfl
      (by simp [Allowed, PreAddOk]))) (Step.answer s1 0 d0 (by simp [s1]) rfl)
  let s3 : State := { s2 with file := upd s2.file 7 (some 0), pending := some (0, 7) }
  have r3 : Reach Config.post wOk s3 :=
    Reach.step r2 (Step.txPublish s2 0 d1 7 rfl ⟨by simp [s2], rfl, rfl, rfl, rfl, rfl⟩ rfl rfl rfl)
  let d2 : Dec := { d1 with claim := some 7 }
  let s4 : State := { s3 with cue := upd s3.cue 0 (some d2), pending := none }
  have r4 : Reach Config.post wOk s4 :=
    Reach.step r3 (Step.txCommit s3 0 d1 7 rfl rfl (by simp [s3, s2]) rfl)
  have r5 := Reach.step r4 (Step.complete s4 0 d2 7 (by simp [s4]) rfl (Or.inr (by simp [s4, s3])))
  exact ⟨_, r5, { d2 with claim := none, done := true }, by simp, rfl⟩

/-- [escalation] を reply で cur へ移せる -/
theorem post_progress_reply : ∃ s, Reach Config.post wOk s ∧ s.replied 0 = true ∧
    ∃ x, s.msgs 0 = some x ∧ x.box = .cur ∧ x.tag = .escalation ∧ x.res = .replied := by
  let s1 : State := { init with msgs := upd init.msgs 0 (some (newMsg .escalation 2)) }
  have r1 : Reach Config.post wOk s1 := Reach.step Reach.init (Step.send init 0 .escalation 2 rfl)
  have r2 := Reach.step r1 (Step.reply s1 0 (newMsg .escalation 2) (by simp [s1]) (by simp [newMsg]) rfl)
  exact ⟨_, r2, by simp [upd], toCur (newMsg .escalation 2) .replied, by simp, rfl, rfl, rfl⟩

/-! ### 状態の一元化 -/

theorem post_recovery (w : World) : RecoveryProp Config.post w := by
  intro s _; simp [recovery, Config.post]

/-! ### issue 41 の穴 (crash 後の token、newOnly の fire、E5 の見せた内容) -/

/-- 穴 1 の反例の前半: 修正後の形でも、ファイルの公開後・commit 前に crash すると、どの claim にも
対応しない token ファイルが残る -/
theorem post_orphan_reachable : ∃ s, Reach Config.post wOk s ∧ s.file 7 = some 0 ∧ s.pending = none ∧
    ∃ x, s.cue 0 = some x ∧ x.claim = none := by
  let d0 := newDec wOk 1 .own
  let s1 : State := { init with cue := upd init.cue 0 (some d0) }
  let d1 : Dec := { d0 with answered := true }
  let s2 : State := { s1 with cue := upd s1.cue 0 (some d1) }
  have r2 : Reach Config.post wOk s2 :=
    Reach.step (Reach.step Reach.init (Step.add init 1 0 .own .rawQuoted rfl
      (by simp [Allowed, PreAddOk]))) (Step.answer s1 0 d0 (by simp [s1]) rfl)
  let s3 : State := { s2 with file := upd s2.file 7 (some 0), pending := some (0, 7) }
  have r3 : Reach Config.post wOk s3 :=
    Reach.step r2 (Step.txPublish s2 0 d1 7 rfl ⟨by simp [s2], rfl, rfl, rfl, rfl, rfl⟩ rfl rfl rfl)
  have r4 := Reach.step r3 (Step.fault (Fault.crash s3))
  exact ⟨_, r4, by simp [s3], rfl, d1, by simp [s3, s2], rfl⟩

/-- 穴 1: どの claim にも pending にも無い token ファイルは、次の claim の掃除で消せる -/
theorem post_orphan_sweepable (w : World) (s : State) (t : Tok) (d : DId)
    (hf : s.file t = some d) (hp : s.pending ≠ some (d, t))
    (hc : ∀ d' x, s.cue d' = some x → x.claim ≠ some t) :
    ∃ s', Step Config.post w s s' ∧ s'.file t = none :=
  ⟨_, Step.sweep s t d rfl hf hp hc, by simp⟩

/-- 穴 2 の反例: 見ていない msg が new/ に無いまま fire でき、行き先は watchDie と同じ状態になる -/
theorem pre_spurious_fire : ∃ s s', Reach Config.pre wOk s ∧ Step Config.pre wOk s s' ∧
    s.armed = true ∧ s' = { s with armed := false } := by
  have r1 : Reach Config.pre wOk { init with armed := true, msgs := unskip init.msgs } :=
    Reach.step Reach.init (Step.rearm (Rearm.arm init false (by simp [Config.pre])))
  refine ⟨_, _, r1, Step.fire _ rfl, rfl, ?_⟩
  simp only [State.mk.injEq, true_and, and_true]
  funext i; rfl

/-- 穴 2: 通知は、new/ にあって seen に無い msg を少なくとも 1 件見せる -/
theorem notify_shows_unseen : NotifyProp := by
  intro s s' h
  cases h with
  | fire _ hu =>
    obtain ⟨m, x, hx, hb, hk⟩ := hu
    exact ⟨m, x, hx, hb, hk, by simp [unseenIds, hx, hb, hk]⟩

/-- 穴 3: 人の発言で new/ を見せない形では E5 が崩れる。証人は既に一度見せた msg で、
旧 E5 (結果の状態の surfaced を見る) ではこの証人が条件を満たしてしまっていた -/
theorem e5_needs_humanSurface : ¬ E5Prop { Config.post with humanSurface := false } wOk := by
  intro h
  let c : Config := { Config.post with humanSurface := false }
  let s1 : State := { init with msgs := upd init.msgs 0 (some (newMsg .completion 2)) }
  have r1 : Reach c wOk s1 := Reach.step Reach.init (Step.send init 0 .completion 2 rfl)
  have hn : HasNew s1 := ⟨0, newMsg .completion 2, by simp [s1], rfl⟩
  have r2 := Reach.step r1 (Step.rearm (Rearm.reveal s1 rfl hn))
  have hx : (showNew s1.msgs) 0 = some { newMsg .completion 2 with surfaced := true } := by
    simp [showNew, mapMsgs, s1, newMsg]
  have := (h _ r2 0 _ hx rfl).2 _ (Human.speak _)
  simp [c, Config.post] at this

end Handoff
