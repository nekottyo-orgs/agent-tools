/-
目的: bash-gates/src/main.rs の判定部 (has_word / triggered / select) と合成部
      (joined / merge) を写したモデルと、その性質の証明。

範囲 (why):
  - 証明の対象はこのモデル。Rust との一致は main.rs の #[cfg(test)] が、Vectors.lean の
    `main` が書き出す突き合わせ表 (vectors.txt) を読んで関数単位で確かめる
    (表の作り直しは repo root で `d=$(mktemp -d) && lean -q -R bash-gates/lean -o "$d/BashGates.olean" bash-gates/lean/BashGates.lean && LEAN_PATH="$d" lean --run bash-gates/lean/Vectors.lean`)
  - spawn・timeout・kill・stale 時の exec・JSON の解釈は対象外。command は「文字列なら
    そのバイト列、文字列でなければ none」として受ける (serde と Lean の JSON は重複キー等で
    挙動が違いうるため、解釈はモデルに入れない)
  - 文字列はバイト列 (List UInt8) で扱う。Rust の has_word がバイト添字だから。quote 除去は
    Rust では char 単位だが、UTF-8 の多バイト列は ASCII のバイトを含まないので同じ結果になる
  - Mathlib は使わない (core の tactic で足りる)
-/

namespace BashGates

abbrev Bytes := List UInt8

/-- `[A-Za-z0-9_.-]`。Rust の is_wordish -/
def isWordish (b : UInt8) : Bool :=
  let n := b.toNat
  (48 ≤ n && n ≤ 57) || (65 ≤ n && n ≤ 90) || (97 ≤ n && n ≤ 122) || n == 95 || n == 46 || n == 45

/-- Rust の has_word を添字の走査のまま写したもの -/
def hasWord (s w : Bytes) : Bool :=
  !w.isEmpty && decide (w.length ≤ s.length) &&
  (List.range (s.length - w.length + 1)).any fun i =>
    (s.drop i).take w.length == w &&
    (i == 0 || !isWordish (s.getD (i - 1) 0)) &&
    (i + w.length == s.length || !isWordish (s.getD (i + w.length) 0))

/-- Rust の str::contains。空の substring は常に当たる -/
def containsB (t sub : Bytes) : Bool :=
  (List.range (t.length + 1)).any fun i => (t.drop i).take sub.length == sub

structure Entry where
  name : String
  words : List Bytes
  substrings : List Bytes
deriving Repr, DecidableEq

def triggered (e : Entry) (texts : List Bytes) : Bool :=
  texts.any fun t => e.words.any (hasWord t) || e.substrings.any (containsB t)

/-- `\` `"` `'` を消す -/
def strip (t : Bytes) : Bytes :=
  t.filter fun b => !(b == 92 || b == 34 || b == 39)

/-- command が文字列でなければ (none) 全員を起動する -/
def select (es : List Entry) : Option Bytes → List Entry
  | none => es
  | some raw => es.filter fun e => triggered e [raw, strip raw]

structure Outcome where
  name : String
  decision : Option String
  reason : String
  ctx : Option String
  sys : Option String
deriving Repr, DecidableEq

/-- 合成結果。ctx / sys が空文字なら Rust はその field を出さない -/
structure Out where
  decision : Option String
  reason : Option String
  ctx : String
  sys : String
deriving Repr, DecidableEq

/-- 同じ理由文は最初に出た子だけを残す -/
def dedupe (xs : List Outcome) : List Outcome :=
  xs.foldl (fun acc x => if acc.any (·.reason == x.reason) then acc else acc ++ [x]) []

def joined (xs : List Outcome) : String :=
  match dedupe xs with
  | [u] => u.reason
  | us => "\n\n".intercalate (us.map fun u => s!"[{u.name}]\n{u.reason}")

def merge (os : List (Option Outcome)) : Option Out :=
  let rs := os.filterMap id
  let deny := rs.filter (·.decision == some "deny")
  let ask := rs.filter (·.decision == some "ask")
  let ctx := "\n\n".intercalate (rs.filterMap (·.ctx))
  let sys := "\n\n".intercalate (rs.filterMap (·.sys))
  if !deny.isEmpty then some ⟨some "deny", some (joined deny), ctx, sys⟩
  else if !ask.isEmpty then some ⟨some "ask", some (joined ask), ctx, sys⟩
  else if ctx.isEmpty && sys.isEmpty then none
  else some ⟨none, none, ctx, sys⟩

/-! ## 性質 -/

/-- T1: has_word は「前後が語を作らない文字か端である w の出現がある」と同値 -/
theorem hasWord_spec (s w : Bytes) :
    hasWord s w = true ↔
      w ≠ [] ∧ ∃ p q, s = p ++ w ++ q ∧
        (∀ b, p.getLast? = some b → isWordish b = false) ∧
        (∀ b, q.head? = some b → isWordish b = false) := by
  unfold hasWord
  simp only [Bool.and_eq_true, Bool.not_eq_true', List.isEmpty_eq_false_iff, decide_eq_true_eq,
    List.any_eq_true, List.mem_range, Bool.or_eq_true, beq_iff_eq, ne_eq]
  constructor
  · rintro ⟨⟨hw, hlen⟩, i, hi, ⟨htake, hl⟩, hr⟩
    refine ⟨hw, s.take i, s.drop (i + w.length), ?_, ?_, ?_⟩
    · conv => lhs; rw [← List.take_append_drop i s, ← List.take_append_drop w.length (s.drop i)]
      rw [htake, List.drop_drop, List.append_assoc]
    · intro b hb
      have hi0 : i ≠ 0 := by rintro rfl; simp at hb
      rcases hl with hl | hl
      · exact absurd hl hi0
      · have hi1 : i - 1 < i := by omega
        rw [List.getLast?_eq_getElem?, List.length_take, List.getElem?_take] at hb
        have hmin : min i s.length - 1 = i - 1 := by omega
        simp only [hmin, hi1, ite_true] at hb
        rw [List.getD_eq_getElem?_getD, hb] at hl
        simpa using hl
    · intro b hb
      rcases hr with hr | hr
      · rw [hr] at hb; simp at hb
      · rw [List.head?_drop] at hb
        rw [List.getD_eq_getElem?_getD, hb] at hr
        simpa using hr
  · rintro ⟨hw, p, q, rfl, hp, hq⟩
    refine ⟨⟨hw, by simp only [List.length_append]; omega⟩, p.length,
      by simp only [List.length_append]; omega, ⟨?_, ?_⟩, ?_⟩
    · rw [List.append_assoc, List.drop_left, List.take_left]
    · cases h : p.getLast? with
      | none => left; simpa using h
      | some b =>
        right
        have hne : p ≠ [] := by intro h'; subst h'; simp at h
        have hlt : p.length - 1 < p.length := by
          have := List.length_pos_iff.mpr hne; omega
        rw [List.getD_eq_getElem?_getD, List.append_assoc, List.getElem?_append_left hlt,
          ← List.getLast?_eq_getElem?, h]
        exact hp b h
    · cases h : q.head? with
      | none => left; rw [List.head?_eq_none_iff] at h; subst h; simp
      | some b =>
        right
        rw [List.getD_eq_getElem?_getD, List.getElem?_append_right (by simp),
          List.length_append, Nat.sub_self, ← List.head?_eq_getElem?, h]
        exact hq b h

/-- T2a: command が文字列でなければ全員を起動する -/
theorem select_none (es : List Entry) : select es none = es := by
  rfl

/-- T2b: 選ぶのは元の並びを保った部分列だけ -/
theorem select_sublist (es : List Entry) (c : Option Bytes) : (select es c).Sublist es := by
  cases c with
  | none => exact List.Sublist.refl es
  | some raw => exact List.filter_sublist

/-- T3: 生の command と quote を消した command のどちらかで当たる entry は必ず起動し、
    どちらでも当たらない entry は起動しない -/
theorem mem_select (es : List Entry) (raw : Bytes) (e : Entry) :
    e ∈ select es (some raw) ↔ e ∈ es ∧ (triggered e [raw] = true ∨ triggered e [strip raw] = true) := by
  simp only [select, List.mem_filter, triggered, List.any_cons, List.any_nil, Bool.or_false,
    Bool.or_eq_true]

/-- os の中に decision が d の子がいるかどうかが、merge の分岐条件 (filter の非空) と一致する -/
theorem pick_isEmpty (os : List (Option Outcome)) (d : String) :
    ((os.filterMap id).filter (·.decision == some d)).isEmpty = true ↔
      ∀ x, some x ∈ os → x.decision ≠ some d := by
  simp [List.isEmpty_iff, List.filter_eq_nil_iff, List.mem_filterMap]

theorem pick_nonempty (os : List (Option Outcome)) (r : Outcome) (d : String)
    (hr : some r ∈ os) (hd : r.decision = some d) :
    ((os.filterMap id).filter (·.decision == some d)).isEmpty = false := by
  cases h : ((os.filterMap id).filter (·.decision == some d)).isEmpty with
  | false => rfl
  | true => exact absurd hd ((pick_isEmpty os d).mp h r hr)

/-- T4a: deny が 1 件でもあれば deny -/
theorem merge_deny (os : List (Option Outcome)) (r : Outcome)
    (hr : some r ∈ os) (hd : r.decision = some "deny") :
    ∃ o, merge os = some o ∧ o.decision = some "deny" := by
  have h := pick_nonempty os r _ hr hd
  unfold merge
  simp only [h, Bool.not_false, ite_true]
  exact ⟨_, rfl, rfl⟩

/-- T4b: deny が無く ask があれば ask -/
theorem merge_ask (os : List (Option Outcome)) (r : Outcome)
    (hr : some r ∈ os) (ha : r.decision = some "ask")
    (hn : ∀ x, some x ∈ os → x.decision ≠ some "deny") :
    ∃ o, merge os = some o ∧ o.decision = some "ask" := by
  have hd := (pick_isEmpty os "deny").mpr hn
  have h := pick_nonempty os r _ hr ha
  unfold merge
  simp only [hd, h, Bool.not_false, Bool.not_true, ite_true, Bool.false_eq_true, ite_false]
  exact ⟨_, rfl, rfl⟩

/-- T4c: deny と ask 以外の decision (allow 等) は出さない -/
theorem merge_decision (os : List (Option Outcome)) (o : Out) (h : merge os = some o) :
    o.decision = none ∨ o.decision = some "deny" ∨ o.decision = some "ask" := by
  unfold merge at h
  dsimp only at h
  split at h
  · cases h; simp
  · split at h
    · cases h; simp
    · split at h
      · cases h
      · cases h; simp

/-- dedupe は先に集めたものを消さない -/
theorem dedupe_go_mono (acc xs : List Outcome) (u : Outcome) (hu : u ∈ acc) :
    u ∈ xs.foldl (fun acc x => if acc.any (·.reason == x.reason) then acc else acc ++ [x]) acc := by
  induction xs generalizing acc with
  | nil => exact hu
  | cons x xs ih =>
    simp only [List.foldl_cons]
    apply ih
    split
    · exact hu
    · exact List.mem_append_left _ hu

/-- 入力のどの理由文も、dedupe の結果のどれかと同じ理由文として残る -/
theorem dedupe_go_cover (acc xs : List Outcome) (x : Outcome) (hx : x ∈ xs) :
    ∃ u ∈ xs.foldl (fun acc x => if acc.any (·.reason == x.reason) then acc else acc ++ [x]) acc,
      u.reason = x.reason := by
  induction xs generalizing acc with
  | nil => cases hx
  | cons y ys ih =>
    simp only [List.foldl_cons]
    rcases List.mem_cons.mp hx with rfl | hx
    · split
      · rename_i h
        obtain ⟨u, hu, he⟩ := List.any_eq_true.mp h
        exact ⟨u, dedupe_go_mono _ _ _ hu, by simpa using he⟩
      · exact ⟨x, dedupe_go_mono _ _ _ (by simp), rfl⟩
    · exact ih _ hx

theorem infix_intercalate (sep x : List Char) (l : List (List Char)) (hx : x ∈ l) :
    x <:+: sep.intercalate l := by
  induction l with
  | nil => cases hx
  | cons a l ih =>
    cases l with
    | nil =>
      rw [List.mem_singleton.mp hx]
      simp [List.intercalate]
    | cons b l =>
      rw [List.intercalate_cons_cons]
      rcases List.mem_cons.mp hx with rfl | hx
      · exact (List.prefix_append _ _).isInfix.trans
          ((List.prefix_append _ _).isInfix)
      · exact (ih hx).trans (List.suffix_append _ _).isInfix

/-- joined は入力のどの理由文も部分列として含む -/
theorem joined_keeps (xs : List Outcome) (x : Outcome) (hx : x ∈ xs) :
    x.reason.toList <:+: (joined xs).toList := by
  obtain ⟨u, hu, he⟩ := dedupe_go_cover [] xs x hx
  unfold joined
  split
  · rename_i v hv
    have : u = v := by
      have := hu; unfold dedupe at hv; rw [hv] at this; simpa using this
    subst this
    rw [he]
    exact List.infix_refl _
  · rw [String.toList_intercalate]
    apply List.IsInfix.trans _ (infix_intercalate _ _ _ (List.mem_map_of_mem
      (List.mem_map_of_mem (f := fun u : Outcome => s!"[{u.name}]\n{u.reason}") hu)))
    show x.reason.toList <:+: ("[" ++ u.name ++ "]\n" ++ u.reason).toList
    rw [he]
    simp only [String.toList_append]
    exact (List.suffix_append _ _).isInfix

/-- T5: deny した子の理由文は、合成後の理由文に部分列としてすべて残る -/
theorem merge_keeps_deny_reason (os : List (Option Outcome)) (r : Outcome)
    (hr : some r ∈ os) (hd : r.decision = some "deny") :
    ∃ o rsn, merge os = some o ∧ o.reason = some rsn ∧ r.reason.toList <:+: rsn.toList := by
  have h := pick_nonempty os r _ hr hd
  unfold merge
  simp only [h, Bool.not_false, ite_true]
  refine ⟨_, _, rfl, rfl, joined_keeps _ _ ?_⟩
  simp [List.mem_filter, List.mem_filterMap, hr, hd]

/-- T6: 何も返さないのは deny・ask・ctx・sys がどれも無いときだけ -/
theorem merge_none (os : List (Option Outcome)) :
    merge os = none ↔
      (∀ x, some x ∈ os → x.decision ≠ some "deny" ∧ x.decision ≠ some "ask") ∧
      "\n\n".intercalate ((os.filterMap id).filterMap (·.ctx)) = "" ∧
      "\n\n".intercalate ((os.filterMap id).filterMap (·.sys)) = "" := by
  have hd := pick_isEmpty os "deny"
  have ha := pick_isEmpty os "ask"
  unfold merge
  dsimp only
  constructor
  · intro h
    split at h
    · cases h
    · split at h
      · cases h
      · split at h
        · rename_i h1 h2 h3
          simp only [Bool.not_eq_true', Bool.not_eq_false, Bool.and_eq_true, String.isEmpty_iff]
            at h1 h2 h3
          exact ⟨fun x hx => ⟨hd.mp h1 x hx, ha.mp h2 x hx⟩, h3⟩
        · cases h
  · rintro ⟨hx, hc, hs⟩
    have h1 := hd.mpr fun x h => (hx x h).1
    have h2 := ha.mpr fun x h => (hx x h).2
    simp only [h1, h2, hc, hs, Bool.not_true, Bool.false_eq_true, ite_false]
    rfl

end BashGates

-- 証明が sorry や native_decide に頼っていないことの確認 (propext / Classical.choice / Quot.sound だけが出る)
#print axioms BashGates.hasWord_spec
#print axioms BashGates.select_none
#print axioms BashGates.select_sublist
#print axioms BashGates.mem_select
#print axioms BashGates.merge_deny
#print axioms BashGates.merge_ask
#print axioms BashGates.merge_decision
#print axioms BashGates.merge_keeps_deny_reason
#print axioms BashGates.merge_none
