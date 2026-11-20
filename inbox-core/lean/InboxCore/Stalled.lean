import Aeneas
import InboxCore.InboxCore
open Aeneas Std Result

/-! Aeneas が生成した inbox_core.stalled (停止の検出) が、Nat の上に置いた仕様 stallSpec に一致することの証明。
仕様は U64 の溢れを持たない。生成物の溢れは、計算する枝 (idle で起点がある) にだけ前提を置き、
前提が破れると fail を返すことを別に示す -/
namespace InboxProofs

/-! ## U64 の演算の ok / fail

生成物の `*` と `+` は範囲外で fail .integerOverflow を返す。step* の spec でなく等式で使うので、ここで等式に直す -/

theorem u64_mul_ok (x y : Std.U64) (h : x.val * y.val ≤ U64.max) :
    ∃ z : Std.U64, x * y = ok z ∧ z.val = x.val * y.val := by
  have hb : x.val * y.val < 18446744073709551616 := by rw [U64.max_eq] at h; omega
  refine ⟨UScalar.ofNatCore (x.val * y.val) (by simp; omega), ?_, rfl⟩
  show UScalar.tryMk _ _ = _
  simp [UScalar.tryMk, UScalar.tryMkOpt, Result.ofOption, hb]

theorem u64_add_ok (x y : Std.U64) (h : x.val + y.val ≤ U64.max) :
    ∃ z : Std.U64, x + y = ok z ∧ z.val = x.val + y.val := by
  have hb : x.val + y.val < 18446744073709551616 := by rw [U64.max_eq] at h; omega
  refine ⟨UScalar.ofNatCore (x.val + y.val) (by simp; omega), ?_, rfl⟩
  show UScalar.tryMk _ _ = _
  simp [UScalar.tryMk, UScalar.tryMkOpt, Result.ofOption, hb]

theorem u64_mul_fail (x y : Std.U64) (h : U64.max < x.val * y.val) :
    x * y = (fail .integerOverflow : Result Std.U64) := by
  have hb : ¬ x.val * y.val < 18446744073709551616 := by rw [U64.max_eq] at h; omega
  show UScalar.tryMk _ _ = _
  simp [UScalar.tryMk, UScalar.tryMkOpt, Result.ofOption, hb]

theorem u64_add_fail (x y : Std.U64) (h : U64.max < x.val + y.val) :
    x + y = (fail .integerOverflow : Result Std.U64) := by
  have hb : ¬ x.val + y.val < 18446744073709551616 := by rw [U64.max_eq] at h; omega
  show UScalar.tryMk _ _ = _
  simp [UScalar.tryMk, UScalar.tryMkOpt, Result.ofOption, hb]

/-! ## 仕様

起点 t0 は report と assign の新しい方 (どちらも無ければ 0)。lib.rs の Origin / Stall と同じ形を Nat の上に持つ -/

inductive SOrigin | report | assign
  deriving DecidableEq, Repr

inductive SStall | active | due (t : Nat) | stalled (o : SOrigin)
  deriving DecidableEq, Repr

/-- idle でないか起点が無ければ Active、起点 + min 分より前なら Due (その時刻)、それ以外は停止 -/
def stallSpec (idle : Bool) (report assign now min : Nat) : SStall :=
  if idle = false ∨ max report assign = 0 then .active
  else if now < max report assign + min * 60 then .due (max report assign + min * 60)
  else if assign ≤ report then .stalled .report
  else .stalled .assign

def originS : inbox_core.Origin → SOrigin
  | .Report => .report | .Assign => .assign

def stallS : inbox_core.Stall → SStall
  | .Active => .active | .Due t => .due t.val | .Stalled o => .stalled (originS o)

/-! ## 一致 -/

/-- 溢れの前提は、生成物が min * 60 と t0 + min * 60 を計算する枝 (idle で起点がある) だけに置く -/
theorem stalled_refines (f : inbox_core.StallFacts) (now min : Std.U64)
    (hov : f.idle = true → max f.report.val f.assign.val ≠ 0 →
      max f.report.val f.assign.val + min.val * 60 ≤ U64.max) :
    ∃ r, inbox_core.stalled f now min = ok r ∧
      stallS r = stallSpec f.idle f.report.val f.assign.val now.val min.val := by
  rcases f with ⟨idle, rep, asg⟩
  simp only at hov ⊢
  obtain ⟨t0, ht0, he⟩ : ∃ t0 : Std.U64, t0.val = max rep.val asg.val ∧
      (if asg.val ≤ rep.val then (ok rep : Result Std.U64) else ok asg) = ok t0 := by
    by_cases hle : asg.val ≤ rep.val
    · exact ⟨rep, by omega, by simp [hle]⟩
    · exact ⟨asg, by omega, by simp [hle]⟩
  simp only [inbox_core.stalled, ge_iff_le, UScalar.le_equiv]
  rw [he]
  simp only [stallSpec, ← ht0]
  cases idle
  · simp [stallS]
  by_cases hz : t0.val = 0
  · have h0 : t0 = 0#u64 := by simp [UScalar.eq_equiv, hz]
    simp [h0, stallS]
  have h : t0.val + min.val * 60 ≤ U64.max := by have := hov rfl (by omega); omega
  obtain ⟨i, hi, hiv⟩ := u64_mul_ok min 60#u64 (by simp; omega)
  have hi60 : i.val = min.val * 60 := by rw [hiv]; simp
  obtain ⟨j, hj, hjv⟩ := u64_add_ok t0 i (by omega)
  have h0 : t0 ≠ 0#u64 := by intro e; simp [e] at hz
  have hj60 : j.val = t0.val + min.val * 60 := by omega
  by_cases hn : now.val < t0.val + min.val * 60
  · have hn' : now.val < j.val := by omega
    refine ⟨.Due j, ?_, ?_⟩
    · simp [h0, hi, hj, hn']
    · simp [stallS, hz, hn, hj60]
  · have hn' : ¬ now.val < j.val := by omega
    by_cases hle : asg.val ≤ rep.val
    · exact ⟨.Stalled .Report, by simp [h0, hi, hj, hn', hle], by simp [stallS, originS, hz, hn, hle]⟩
    · exact ⟨.Stalled .Assign, by simp [h0, hi, hj, hn', hle], by simp [stallS, originS, hz, hn, hle]⟩

/-- 前提が破れると、乗算と加算のどちらで溢れても fail を返す (Due や停止の誤った値は返さない) -/
theorem stalled_overflow_fails (f : inbox_core.StallFacts) (now min : Std.U64)
    (hidle : f.idle = true) (hz : max f.report.val f.assign.val ≠ 0)
    (hov : U64.max < max f.report.val f.assign.val + min.val * 60) :
    inbox_core.stalled f now min = fail .integerOverflow := by
  rcases f with ⟨idle, rep, asg⟩
  simp only at *
  obtain ⟨t0, ht0, he⟩ : ∃ t0 : Std.U64, t0.val = max rep.val asg.val ∧
      (if asg.val ≤ rep.val then (ok rep : Result Std.U64) else ok asg) = ok t0 := by
    by_cases hle : asg.val ≤ rep.val
    · exact ⟨rep, by omega, by simp [hle]⟩
    · exact ⟨asg, by omega, by simp [hle]⟩
  simp only [inbox_core.stalled, ge_iff_le, UScalar.le_equiv]
  rw [he]
  subst hidle
  have h0 : t0 ≠ 0#u64 := by intro e; simp [e] at ht0; omega
  by_cases hm : min.val * (60#u64).val ≤ U64.max
  · obtain ⟨i, hi, hiv⟩ := u64_mul_ok min 60#u64 hm
    have hi60 : i.val = min.val * 60 := by rw [hiv]; simp
    have hj := u64_add_fail t0 i (by omega)
    simp [h0, hi, hj]
  · have hi := u64_mul_fail min 60#u64 (by omega)
    simp [h0, hi]

/-! ## 系 -/

/-- 完全性: idle で起点があり、起点から min 分経っていれば、stalled は停止を返す。
now は U64 なので、起点 + min*60 ≤ now から溢れないことが従い、溢れの前提は要らない -/
theorem stalled_complete (f : inbox_core.StallFacts) (now min : Std.U64)
    (hidle : f.idle = true) (hpos : 0 < max f.report.val f.assign.val)
    (hnow : max f.report.val f.assign.val + min.val * 60 ≤ now.val) :
    inbox_core.stalled f now min =
      ok (.Stalled (if f.assign.val ≤ f.report.val then .Report else .Assign)) := by
  have hmax : now.val ≤ U64.max := by
    have := now.hBounds; rw [U64.max_eq]; simp [UScalarTy.numBits] at this; omega
  obtain ⟨r, hr, hs⟩ := stalled_refines f now min (fun _ _ => by omega)
  rw [hr, Result.ok.injEq]
  have h1 : ¬ (max f.report.val f.assign.val = 0) := by omega
  have h2 : ¬ (now.val < max f.report.val f.assign.val + min.val * 60) := by omega
  simp only [stallSpec, hidle, h1, h2] at hs
  rcases r with _ | _ | ⟨_ | _⟩ <;> by_cases hle : f.assign.val ≤ f.report.val <;>
    simp_all [stallS, originS]

/-- 安全性: stalled が停止を返すなら、idle で起点があり、起点から min 分経っている。
起点は assign ≤ report のとき Report。溢れは fail になり停止を返さないので、溢れの前提は要らない -/
theorem stalled_sound (f : inbox_core.StallFacts) (now min : Std.U64) (o : inbox_core.Origin)
    (h : inbox_core.stalled f now min = ok (.Stalled o)) :
    f.idle = true ∧ 0 < max f.report.val f.assign.val ∧
      max f.report.val f.assign.val + min.val * 60 ≤ now.val ∧
      (o = .Report ↔ f.assign.val ≤ f.report.val) := by
  by_cases hov : f.idle = true → max f.report.val f.assign.val ≠ 0 →
      max f.report.val f.assign.val + min.val * 60 ≤ U64.max
  · obtain ⟨r, hr, hs⟩ := stalled_refines f now min hov
    rw [h, Result.ok.injEq] at hr
    subst hr
    simp only [stallS] at hs
    unfold stallSpec at hs
    split_ifs at hs with h1 h2 h3
    · cases o <;> simp_all [originS]
      omega
    · cases o <;> simp_all [originS]
      omega
  · simp only [Classical.not_imp, _root_.not_le] at hov
    obtain ⟨hidle, hz, hlt⟩ := hov
    rw [stalled_overflow_fails f now min hidle hz hlt] at h
    exact absurd h fail_not_ok

end InboxProofs

#print axioms InboxProofs.u64_mul_ok
#print axioms InboxProofs.u64_add_ok
#print axioms InboxProofs.u64_mul_fail
#print axioms InboxProofs.u64_add_fail
#print axioms InboxProofs.stalled_refines
#print axioms InboxProofs.stalled_overflow_fails
#print axioms InboxProofs.stalled_complete
#print axioms InboxProofs.stalled_sound
