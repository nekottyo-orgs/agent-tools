import Aeneas
import InboxCore.InboxCore
import InboxCore.Proofs
open Aeneas Std Result

/-! 副作用の列を返す inbox_core.plan の性質。Vec.push は長さが Usize.max に届くと fail するので、
step と違い全域性は push の spec (長さ < Usize.max) を経て示す -/
namespace InboxProofs

theorem plan_total (m : inbox_core.Msg) (op : inbox_core.Op) (o : inbox_core.Obs) :
    inbox_core.plan m op o ⦃ v => v.val.length ≤ 3 ⦄ := by
  obtain ⟨r, hr⟩ := step_total m op o
  unfold inbox_core.plan inbox_core.push_move
  simp only [hr]
  rcases m with ⟨t, snd, p, rs⟩
  cases r <;> cases op <;> cases p <;> step* <;> simp_all

theorem plan_escalation_reply :
    inbox_core.plan ⟨.Escalation, 2#u32, .New, .None⟩ .Reply ⟨.Alive, true, true⟩
      ⦃ v => v.val = [.MoveTo .Cur, .AppendLog .Replied] ⦄ := by
  unfold inbox_core.plan inbox_core.push_move
  simp only [escalation_reply_moves]
  step*

end InboxProofs

#print axioms InboxProofs.plan_total
#print axioms InboxProofs.plan_escalation_reply
