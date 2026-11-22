import InboxCore.InboxCore

/-! #62 の `end_marker::judge` の先頭分岐。後続の差し替え・マージ待ち判定は扱わない。 -/
namespace InboxProofs

open inbox_core (Lane Panes)

inductive EndMarkerPrefix where
  | legacy | stay | complete | waitForDecision | continue

def endMarkerPrefix (typed : Bool) (fr : Lane) (marker waiting : Bool) (panes : Panes) :
    EndMarkerPrefix :=
  if !typed then .legacy else
    match fr with
    | .Done => .stay
    | _ =>
      if marker then .complete else
        match fr, panes with
        | .HumanAction, .Live => if waiting then .waitForDecision else .continue
        | _, _ => .continue

theorem humanActionWaitsIff (typed : Bool) (fr : Lane) (marker waiting : Bool) (panes : Panes) :
    endMarkerPrefix typed fr marker waiting panes = .waitForDecision ↔
      typed = true ∧ fr = .HumanAction ∧ marker = false ∧ waiting = true ∧ panes = .Live := by
  cases typed <;> cases fr <;> cases marker <;> cases waiting <;> cases panes <;>
    simp [endMarkerPrefix]

theorem doneStays (marker waiting : Bool) (panes : Panes) :
    endMarkerPrefix true .Done marker waiting panes = .stay := rfl

theorem markerHasPriority (fr : Lane) (waiting : Bool) (panes : Panes) (active : fr ≠ .Done) :
    endMarkerPrefix true fr true waiting panes = .complete := by
  cases fr <;> simp_all [endMarkerPrefix]

theorem legacyKeepsOldJudge (fr : Lane) (marker waiting : Bool) (panes : Panes) :
    endMarkerPrefix false fr marker waiting panes = .legacy := rfl

end InboxProofs
