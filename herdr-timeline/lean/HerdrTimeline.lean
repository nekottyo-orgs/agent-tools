/-
目的: herdr-timeline の判定と増分読み取りを写したモデルと、その性質の証明。

範囲 (why):
  - 証明の対象はこのモデル。Rust との対応は同じ条件の入力を使う回帰試験で確かめる
  - 文字列は List Char で扱う。core の String の補題は少なく、先頭の空白の除去などを帰納法で示すため
  - 時刻は秒の Nat。transcript の行の解釈 (JSON) と、ファイルの読み取りそのものは対象外。
    読めない入力は none として受ける
  - Mathlib は使わない (core の tactic で足りる)
-/

namespace HerdrTimeline

abbrev Str := List Char

/-! ## イベントと順序 -/

inductive Kind where
  | instr   -- 人の指示
  | msg     -- 他 pane からのメッセージ
  | pr      -- PR リンクの初出
  | report  -- inbox の報告
  deriving DecidableEq, Repr

/-- src は源の種類 (0 = transcript、1 = inbox)、seq は源の中の連番 (transcript は行番号、inbox はファイル名順の位置) -/
structure Event where
  ts : Nat
  src : Nat
  seq : Nat
  kind : Kind
  body : Str
  deriving DecidableEq, Repr

/-- a を b より前 (新しい側) に出すか。(ts, src, seq) の辞書式の降順。同時刻は源と連番で決まる -/
def newerEq (a b : Event) : Bool :=
  a.ts > b.ts || (a.ts == b.ts && (a.src > b.src || (a.src == b.src && a.seq ≥ b.seq)))

def order (es : List Event) : List Event := es.mergeSort newerEq

/-- 並べた結果は元の並べ替え (イベントを失わず増やさない) -/
theorem order_perm (es : List Event) : (order es).Perm es :=
  List.mergeSort_perm es newerEq

/-- key の重複が無い列では、同じ key の 2 要素は同じ要素 -/
theorem eq_of_nodup_map {α β : Type} (f : α → β) :
    ∀ (l : List α), (l.map f).Nodup → ∀ a ∈ l, ∀ b ∈ l, f a = f b → a = b
  | [], _, a, ha, _, _, _ => by simp at ha
  | x :: l, hn, a, ha, b, hb, hab => by
    simp only [List.map_cons, List.nodup_cons, List.mem_map, not_exists, not_and] at hn
    simp only [List.mem_cons] at ha hb
    rcases ha with rfl | ha <;> rcases hb with rfl | hb
    · rfl
    · exact absurd hab.symm (hn.1 b hb)
    · exact absurd hab (hn.1 a ha)
    · exact eq_of_nodup_map f l hn.2 a ha b hb hab

/-- 並べ替えの関係にあり、どちらも R で整列済みで、l1 の中で R が反対称なら等しい -/
theorem eq_of_perm_pairwise {α : Type} {R : α → α → Prop} :
    ∀ {l1 l2 : List α}, l1.Perm l2 → l1.Pairwise R → l2.Pairwise R →
      (∀ a ∈ l1, ∀ b ∈ l1, R a b → R b a → a = b) → l1 = l2
  | [], _, hp, _, _, _ => hp.nil_eq
  | _ :: _, [], hp, _, _, _ => absurd hp.eq_nil (by simp)
  | a :: l1, b :: l2, hp, h1, h2, ha => by
    have hab : a = b := by
      have hb : b ∈ a :: l1 := hp.mem_iff.mpr (by simp)
      have ha' : a ∈ b :: l2 := hp.mem_iff.mp (by simp)
      rcases List.mem_cons.mp hb with hb | hb
      · exact hb.symm
      rcases List.mem_cons.mp ha' with ha' | ha'
      · exact ha'
      exact ha a (by simp) b (by simp [hb])
        (List.rel_of_pairwise_cons h1 hb) (List.rel_of_pairwise_cons h2 ha')
    subst hab
    congr 1
    exact eq_of_perm_pairwise hp.cons_inv h1.of_cons h2.of_cons
      (fun x hx y hy => ha x (by simp [hx]) y (by simp [hy]))

theorem newerEq_trans (a b c : Event) : newerEq a b = true → newerEq b c = true → newerEq a c = true := by
  simp only [newerEq, Bool.or_eq_true, Bool.and_eq_true, beq_iff_eq, decide_eq_true_eq]; omega

theorem newerEq_total (a b : Event) : (newerEq a b || newerEq b a) = true := by
  simp only [newerEq, Bool.or_eq_true, Bool.and_eq_true, beq_iff_eq, decide_eq_true_eq]; omega

theorem newerEq_antisymm (a b : Event) (h1 : newerEq a b = true) (h2 : newerEq b a = true) :
    (a.ts, a.src, a.seq) = (b.ts, b.src, b.seq) := by
  simp only [newerEq, Bool.or_eq_true, Bool.and_eq_true, beq_iff_eq, decide_eq_true_eq] at h1 h2
  simp only [Prod.mk.injEq]; omega

/-- 列挙の順に依らない。(ts, src, seq) が重複しない入力なら、並べ替えた入力から同じ結果になる -/
theorem order_indep (xs ys : List Event) (h : xs.Perm ys)
    (huniq : (xs.map fun e => (e.ts, e.src, e.seq)).Nodup) : order xs = order ys := by
  unfold order
  apply eq_of_perm_pairwise
  · exact (List.mergeSort_perm xs newerEq).trans (h.trans (List.mergeSort_perm ys newerEq).symm)
  · exact List.pairwise_mergeSort newerEq_trans newerEq_total xs
  · exact List.pairwise_mergeSort newerEq_trans newerEq_total ys
  · intro a ha b hb hab hba
    rw [List.mem_mergeSort] at ha hb
    exact eq_of_nodup_map _ xs huniq a ha b hb (newerEq_antisymm a b hab hba)

/-! ## 隣り合う同じ種類のまとめ (×N) -/

def group : List Event → List (Event × Nat)
  | [] => []
  | e :: es =>
    match group es with
    | (g, n) :: rest => if g.kind = e.kind then (e, n + 1) :: rest else (e, 1) :: (g, n) :: rest
    | [] => [(e, 1)]

def total (gs : List (Event × Nat)) : Nat := (gs.map (·.2)).sum

/-- まとめても件数の合計は元のイベント数と等しい -/
theorem group_total (es : List Event) : total (group es) = es.length := by
  induction es with
  | nil => rfl
  | cons e es ih =>
    simp only [group]
    split
    · rename_i g n rest h
      rw [h] at ih
      split <;> simp only [total, List.map_cons, List.sum_cons, List.length_cons] at ih ⊢ <;> omega
    · rename_i h
      rw [h] at ih
      simp only [total, List.map_cons, List.map_nil, List.sum_cons, List.sum_nil,
        List.length_cons] at ih ⊢; omega

/-- 隣り合う 2 行の種類が違う -/
def Adj (gs : List (Event × Nat)) : Prop :=
  ∀ i, (h : i + 1 < gs.length) → (gs[i]'(by omega)).1.kind ≠ (gs[i+1]'h).1.kind

theorem adj_replace_head (x y : Event × Nat) (rest : List (Event × Nat))
    (h : Adj (x :: rest)) (hk : x.1.kind = y.1.kind) : Adj (y :: rest) := by
  intro i hi
  cases i with
  | zero => have := h 0 hi; simp at this ⊢; rw [← hk]; exact this
  | succ i => have := h (i + 1) hi; simpa using this

theorem adj_cons (x y : Event × Nat) (rest : List (Event × Nat))
    (h : Adj (x :: rest)) (hk : y.1.kind ≠ x.1.kind) : Adj (y :: x :: rest) := by
  intro i hi
  cases i with
  | zero => simpa using hk
  | succ i => have := h i (by simp at hi ⊢; omega); simpa using this

theorem adj_group (es : List Event) : Adj (group es) := by
  induction es with
  | nil => intro i h; simp [group] at h
  | cons e es ih =>
    simp only [group]
    split
    · rename_i g n rest hg
      rw [hg] at ih
      split
      · rename_i hk
        exact adj_replace_head (g, n) (e, n + 1) rest ih hk
      · rename_i hk
        exact adj_cons (g, n) (e, 1) rest ih (fun h => hk h.symm)
    · intro i h; simp at h

/-- まとめた結果で隣り合う 2 行は種類が違う -/
theorem group_adjacent (es : List Event) :
    ∀ i, (h : i + 1 < (group es).length) →
      ((group es)[i]'(by omega)).1.kind ≠ ((group es)[i+1]'h).1.kind :=
  adj_group es

/-! ## transcript の行 (要件 1, 2, 4) -/

/-- JSON を解釈した後の 1 行。type は user / pr-link / その他 -/
structure Line where
  ts : Option Nat
  typ : Str
  isMeta : Bool
  isSidechain : Bool
  isCompactSummary : Bool
  content : Option Str  -- message.content が文字列のときだけ some
  pr : Option Str       -- pr-link の URL
  deriving Repr

def isWs (c : Char) : Bool := c == ' ' || c == '\n' || c == '\t' || c == '\r'

def trimLeft (s : Str) : Str := s.dropWhile isWs

/-- 指示として扱わない前置き。判定は先頭の空白を除いてから行う -/
def skipPrefixes : List Str :=
  ["<".toList, "Stop hook feedback:".toList, "This session is being continued".toList]

/-- 貼り付け wrapper。中身の行を指示として出す -/
def pasteOpen : Str := "<pasted_content".toList

inductive Class where
  | herdrMsg   -- [herdr-message] を含む
  | paste      -- 貼り付け。中身を指示として出す
  | instr      -- 人の指示
  | skip
  deriving DecidableEq, Repr

def containsSub (s sub : Str) : Bool :=
  (List.range (s.length + 1)).any fun i => (s.drop i).take sub.length == sub

def herdrTag : Str := "[herdr-message] from ".toList

def classify (s : Str) : Class :=
  let t := trimLeft s
  if containsSub t herdrTag then .herdrMsg
  else if pasteOpen.isPrefixOf t then .paste
  else if t.isEmpty || skipPrefixes.any (·.isPrefixOf t) then .skip
  else .instr

/-- 除外条件。user 行の meta / sidechain / compact summary -/
def excluded (l : Line) : Bool := l.isMeta || l.isSidechain || l.isCompactSummary

def userEvent (i : Nat) (l : Line) : Option Event :=
  match l.ts, l.content with
  | some ts, some c =>
    if excluded l then none
    else match classify c with
      | .herdrMsg => some ⟨ts, 0, i, .msg, c⟩
      | .paste => some ⟨ts, 0, i, .instr, c⟩
      | .instr => some ⟨ts, 0, i, .instr, c⟩
      | .skip => none
  | _, _ => none

/-- PR は URL の初出だけ。seen は既に出した URL。除外行は種類によらず飛ばし、seen にも入れない -/
def lineEvents : Nat → List Str → List Line → List Event
  | _, _, [] => []
  | i, seen, l :: ls =>
    if excluded l then lineEvents (i+1) seen ls
    else if l.typ = "pr-link".toList then
      match l.ts, l.pr with
      | some ts, some u =>
        if u ∈ seen then lineEvents (i+1) seen ls
        else ⟨ts, 0, i, .pr, u⟩ :: lineEvents (i+1) (u :: seen) ls
      | _, _ => lineEvents (i+1) seen ls
    else if l.typ = "user".toList then
      match userEvent i l with
      | some e => e :: lineEvents (i+1) seen ls
      | none => lineEvents (i+1) seen ls
    else lineEvents (i+1) seen ls

/-- transcript を読めなければ none -/
def transcriptEvents (input : Option (List Line)) : List Event :=
  match input with
  | none => []
  | some ls => lineEvents 0 [] ls

/-- 要件 1: 読めない transcript はイベント 0 件 (例外で止まらない) -/
theorem unreadable_empty : transcriptEvents none = [] := rfl

/-- 要件 2: 除外条件に当たる行は、user でも pr-link でもイベントにならず、
    後続の判定 (PR の初出を決める seen) にも影響しない -/
theorem excluded_no_event (i : Nat) (l : Line) (h : excluded l = true) :
    userEvent i l = none ∧
      ∀ seen ls, lineEvents i seen (l :: ls) = lineEvents (i+1) seen ls := by
  refine ⟨?_, ?_⟩
  · unfold userEvent
    split
    · simp [h]
    · rfl
  · intro seen ls
    simp [lineEvents, h]

theorem dropWhile_ws_append (ws s : Str) (h : ∀ c ∈ ws, isWs c = true) :
    (ws ++ s).dropWhile isWs = s.dropWhile isWs := by
  induction ws with
  | nil => rfl
  | cons c cs ih =>
    have hc : isWs c = true := h c (by simp)
    simp [hc]
    exact ih (fun x hx => h x (by simp [hx]))

/-- 要件 4: 判定は先頭の空白を足しても変わらない -/
theorem classify_ws (ws s : Str) (h : ∀ c ∈ ws, isWs c = true) :
    classify (ws ++ s) = classify s := by
  unfold classify trimLeft
  rw [dropWhile_ws_append ws s h]

/-! ## pane への帰属 (要件 3) -/

structure Msg where
  fromPane : Str
  fromSocket : Option Str  -- 旧形式は none
  ts : Nat
  seq : Nat
  tag : Str
  body : Str

structure Pane where
  pane : Str
  socket : Str
  since : Option Nat  -- session の最初の時刻。pane id の再利用に備える

/-- 帰属は (socket, pane id) が一致し、session の開始以降のときだけ。
    from_socket の無い旧形式と、開始時刻 (since) が分からない pane には帰属させない -/
def attributed (p : Pane) (m : Msg) : Bool :=
  m.fromSocket == some p.socket && m.fromPane == p.pane &&
    (match p.since with | none => false | some s => decide (s ≤ m.ts))

theorem attributed_sound (p : Pane) (m : Msg) (h : attributed p m = true) :
    m.fromSocket = some p.socket ∧ m.fromPane = p.pane ∧ ∃ t, p.since = some t ∧ t ≤ m.ts := by
  unfold attributed at h
  simp only [Bool.and_eq_true, beq_iff_eq] at h
  obtain ⟨⟨h1, h2⟩, h3⟩ := h
  refine ⟨h1, h2, ?_⟩
  cases hs : p.since with
  | none => rw [hs] at h3; simp at h3
  | some t => rw [hs] at h3; exact ⟨t, rfl, by simpa using h3⟩

theorem legacy_not_attributed (p : Pane) (m : Msg) (h : m.fromSocket = none) : attributed p m = false := by
  simp [attributed, h]

/-- 開始時刻が分からない pane (空・壊れ・読めない transcript) には、どの報告も帰属しない -/
theorem unknown_since_not_attributed (p : Pane) (m : Msg) (h : p.since = none) : attributed p m = false := by
  simp [attributed, h]

/-! ## 解析キャッシュ (要件 5) -/

structure File where
  key : Nat × Nat  -- (mtime, size)
  lines : Option (List Line)

abbrev Cache := Option ((Nat × Nat) × List Event)

def cachedParse (c : Cache) (f : File) : List Event × Cache :=
  match c with
  | some (k, v) => if k = f.key then (v, c) else
      let v' := transcriptEvents f.lines; (v', some (f.key, v'))
  | none => let v' := transcriptEvents f.lines; (v', some (f.key, v'))

/-- 内容は key (mtime, size) から決まる固定の関数 content で与える。
    「同じ key なら中身も同じ」は mtime と size だけを見る実装の前提で、ここではその前提を
    content という関数の存在として置く (全 File を量化する前提だと、同じ key で lines が違う File が作れて矛盾する) -/
def Consistent (content : Nat × Nat → Option (List Line)) (f : File) : Prop :=
  f.lines = content f.key

/-- キャッシュの中身は、その key の内容の解析結果と等しい -/
def Valid (c : Cache) (content : Nat × Nat → Option (List Line)) : Prop :=
  ∀ k v, c = some (k, v) → v = transcriptEvents (content k)

/-- 同じ mtime と size ならキャッシュの結果は再解析の結果と等しく、キャッシュは正しいまま -/
theorem cached_eq_parse (c : Cache) (f : File) (content : Nat × Nat → Option (List Line))
    (hf : Consistent content f) (hv : Valid c content) :
    (cachedParse c f).1 = transcriptEvents f.lines ∧ Valid (cachedParse c f).2 content := by
  unfold cachedParse
  split
  · rename_i k v
    split
    · rename_i hk
      refine ⟨?_, hv⟩
      rw [hv k v rfl, hf, hk]
    · refine ⟨rfl, ?_⟩
      intro k' v' hc
      simp at hc
      obtain ⟨rfl, rfl⟩ := hc
      rw [hf]
  · refine ⟨rfl, ?_⟩
    intro k' v' hc
    simp at hc
    obtain ⟨rfl, rfl⟩ := hc
    rw [hf]

/-- 前提は空虚でない: どの content にも、それと整合する File があり、空のキャッシュは Valid -/
theorem cached_premises_satisfiable (content : Nat × Nat → Option (List Line)) (k : Nat × Nat) :
    Consistent content ⟨k, content k⟩ ∧ Valid none content :=
  ⟨rfl, fun _ _ h => by cases h⟩

/-! ## session id の検証 (要件 8) -/

def allowed (c : Char) : Bool :=
  c.isAlphanum || c == '.' || c == '_' || c == '-'

/-- 空でなく、許した文字だけで、先頭が . でない (. と .. を弾く) -/
def validSid (s : Str) : Bool :=
  match s with
  | [] => false
  | c :: _ => c != '.' && s.all allowed

/-- 受け付けた session_id のファイル名は区切りを含まず、. や .. にもならない。
    よって projects/<dir>/<sid>.jsonl は projects/<dir> の外を指さない -/
theorem validSid_safe (s : Str) (h : validSid s = true) :
    (∀ c ∈ s ++ ".jsonl".toList, c ≠ '/') ∧ s ++ ".jsonl".toList ≠ ".".toList ∧ s ++ ".jsonl".toList ≠ "..".toList := by
  match s, h with
  | c :: cs, h =>
    simp only [validSid, Bool.and_eq_true, bne_iff_ne, ne_eq, List.all_eq_true] at h
    obtain ⟨hdot, hall⟩ := h
    refine ⟨?_, ?_, ?_⟩
    · intro x hx
      simp only [List.mem_append] at hx
      rcases hx with hx | hx
      · have := hall x hx
        intro hxs; subst hxs; simp [allowed] at this
      · intro hxs; subst hxs; revert hx; decide
    · intro heq; simp at heq
    · intro heq; simp at heq; exact hdot heq.1

/-! ## brief と進捗 (要件 9 と step) -/

/-- 最後に指示された brief を正とする (出し直しに追随する)。xs は指示の列、各要素はその指示が含む brief -/
def lastBrief (xs : List (Option Str)) : Option Str := (xs.filterMap id).getLast?

/-- 要件 9: brief を含む指示が後から来たら、その brief に切り替わる -/
theorem lastBrief_reissue (xs : List (Option Str)) (b : Str) :
    lastBrief (xs ++ [some b]) = some b := by
  simp [lastBrief]

/-- brief を含まない指示は表示する brief を変えない -/
theorem lastBrief_skip (xs : List (Option Str)) :
    lastBrief (xs ++ [none]) = lastBrief xs := by
  simp [lastBrief]

/-- [progress] 1 通。names はその時点で済んだ step 名の一覧 (差分でなく丸ごと) -/
structure Progress where
  ts : Nat
  seq : Nat
  names : List Str

def progNewerEq (a b : Progress) : Bool := a.ts > b.ts || (a.ts == b.ts && a.seq ≥ b.seq)

/-- 最新の [progress]。(ts, seq) が最大のもの -/
def latest : List Progress → Option Progress
  | [] => none
  | p :: ps =>
    match latest ps with
    | none => some p
    | some q => if progNewerEq p q then some p else some q

inductive Status where
  | unreported                         -- [progress] が 1 通も無い
  | reported (done : List Str) (unknown : Nat)  -- 済み step (brief の順) と brief に無い名前の数
  deriving DecidableEq, Repr

/-- brief に無い step 名は済みに数えず、件数だけ残す -/
def status (steps : List Str) (ps : List Progress) : Status :=
  match latest ps with
  | none => .unreported
  | some p => .reported (steps.filter (· ∈ p.names)) (p.names.filter (· ∉ steps)).length

/-- 済み集合は brief の step 集合の部分集合で、N ≤ M -/
theorem done_subset (steps : List Str) (ps : List Progress) (done : List Str) (u : Nat)
    (h : status steps ps = .reported done u) : (∀ x ∈ done, x ∈ steps) ∧ done.length ≤ steps.length := by
  unfold status at h
  split at h
  · cases h
  · simp at h
    obtain ⟨rfl, -⟩ := h
    exact ⟨fun x hx => (List.mem_filter.mp hx).1, List.length_filter_le _ _⟩

/-- [progress] が 1 通も無いことと、0 step 済みは区別される -/
theorem unreported_ne_zero (_steps : List Str) (u : Nat) : Status.unreported ≠ Status.reported [] u := by
  intro h; cases h

theorem unreported_iff (steps : List Str) (ps : List Progress) :
    status steps ps = .unreported ↔ ps = [] := by
  constructor
  · intro h
    cases ps with
    | nil => rfl
    | cons p ps =>
      exfalso
      have hs : (latest (p :: ps)).isSome := by
        simp only [latest]; split <;> (try split) <;> rfl
      unfold status at h
      revert hs h
      cases latest (p :: ps) <;> simp
  · intro h; subst h; rfl

theorem latest_spec : ∀ (ps : List Progress) (p : Progress), latest ps = some p →
    p ∈ ps ∧ ∀ q ∈ ps, progNewerEq p q = true
  | [], p, h => by simp [latest] at h
  | x :: ps, p, h => by
    simp only [latest] at h
    split at h
    · rename_i hl
      cases h
      have : ps = [] := by
        cases ps with
        | nil => rfl
        | cons y ys =>
          simp only [latest] at hl
          split at hl <;> (try split at hl) <;> cases hl
      subst this
      simp [progNewerEq]
    · rename_i q hl
      obtain ⟨hq, hmax⟩ := latest_spec ps q hl
      split at h <;> cases h
      · rename_i hxq
        refine ⟨by simp, ?_⟩
        intro r hr
        simp only [List.mem_cons] at hr
        rcases hr with rfl | hr
        · simp [progNewerEq]
        · have := hmax r hr
          simp only [progNewerEq, Bool.or_eq_true, Bool.and_eq_true, beq_iff_eq,
            decide_eq_true_eq] at hxq this ⊢
          omega
      · rename_i hxq
        refine ⟨by simp [hq], ?_⟩
        intro r hr
        simp only [List.mem_cons] at hr
        rcases hr with rfl | hr
        · simp only [progNewerEq, Bool.or_eq_true, Bool.and_eq_true, beq_iff_eq,
            decide_eq_true_eq] at hxq ⊢
          omega
        · exact hmax r hr

theorem latest_none_iff (ps : List Progress) : latest ps = none ↔ ps = [] := by
  cases ps with
  | nil => simp [latest]
  | cons p ps =>
    simp only [latest, reduceCtorEq, iff_false]
    split <;> (try split) <;> simp

/-- 表示する済み集合は届く順に依らない ((ts, seq) が重複しないとき) -/
theorem status_indep (steps : List Str) (xs ys : List Progress) (h : xs.Perm ys)
    (huniq : (xs.map fun p => (p.ts, p.seq)).Nodup) : status steps xs = status steps ys := by
  have hl : latest xs = latest ys := by
    cases hx : latest xs with
    | none =>
      rw [latest_none_iff] at hx; subst hx
      rw [← List.Perm.nil_eq h]; rfl
    | some p =>
      cases hy : latest ys with
      | none =>
        rw [latest_none_iff] at hy; subst hy
        rw [List.Perm.eq_nil h] at hx; simp [latest] at hx
      | some q =>
        obtain ⟨hp, hpmax⟩ := latest_spec xs p hx
        obtain ⟨hq, hqmax⟩ := latest_spec ys q hy
        have hq' : q ∈ xs := h.mem_iff.mpr hq
        have hp' : p ∈ ys := h.mem_iff.mp hp
        have h1 := hpmax q hq'
        have h2 := hqmax p hp'
        congr 1
        apply eq_of_nodup_map _ xs huniq p hp q hq'
        simp only [progNewerEq, Bool.or_eq_true, Bool.and_eq_true, beq_iff_eq,
          decide_eq_true_eq] at h1 h2
        simp only [Prod.mk.injEq]; omega
  unfold status
  rw [hl]


/-! ## 未処理報告の窓口 -/

def validPaneId (id : Str) : Bool := !id.isEmpty

/-- gather の観測判定。配列を読めなければ none、欠落や非文字列の ID は空文字列。
    ID の整形は Rust 側で行う。 -/
def paneListObserved (ids : Option (List Str)) : Bool :=
  match ids with
  | none => false
  | some ids => ids.all validPaneId

theorem paneListObserved_iff (ids : Option (List Str)) :
    paneListObserved ids = true ↔ ∃ rows, ids = some rows ∧ ∀ id ∈ rows, id ≠ [] := by
  cases ids <;> simp [paneListObserved, validPaneId, List.all_eq_true]

example : paneListObserved (some []) = true := rfl

/-- gather が Snapshot に残す pane の ID。観測できなかった一覧でも有効な行は残す。 -/
def retainedPaneIds (ids : List Str) : List Str := ids.filter validPaneId

theorem retainedPaneIds_iff (ids : List Str) (id : Str) :
    id ∈ retainedPaneIds ids ↔ id ∈ ids ∧ id ≠ [] := by
  simp [retainedPaneIds, validPaneId]

structure RunPane where
  id : Str
  socket : Str
  label : Str
  deriving DecidableEq, Repr

inductive RunWindow where
  | active (label : Str)
  | absent
  | unknown
  | closed
  deriving DecidableEq, Repr

def windowMatches (id socket : Str) (pane : RunPane) : Bool :=
  pane.id == id && pane.socket == socket

/-- Rust の read_runs に対応。socket は正規化済みで、observed は一覧が取得でき全 ID を読めたときだけ真。 -/
def runWindow (id socket : Str) (closed observed : Bool) (panes : List RunPane) : RunWindow :=
  if id.isEmpty && closed then .closed
  else if id.isEmpty || id == "unknown".toList || !observed then .unknown
  else match panes.find? (windowMatches id socket) with
    | some pane => .active (if pane.label.isEmpty then pane.id else pane.label)
    | none => .absent

theorem runWindow_absent_requires_observation (id socket : Str) (closed observed : Bool)
    (panes : List RunPane) (h : runWindow id socket closed observed panes = .absent) :
    observed = true ∧ ∀ p ∈ panes, p.id ≠ id ∨ p.socket ≠ socket := by
  unfold runWindow at h
  split at h <;> try contradiction
  split at h <;> try contradiction
  rename_i hn
  split at h <;> try contradiction
  rename_i hf
  have ho : observed = true := by
    cases observed <;> simp_all
  refine ⟨ho, ?_⟩
  intro p hp
  have hm := List.find?_eq_none.mp hf p hp
  by_cases hi : p.id = id
  · right
    intro hs
    simp [windowMatches, hi, hs] at hm
  · exact Or.inl hi

theorem runWindow_active_requires_same_socket (id socket label : Str) (closed observed : Bool)
    (panes : List RunPane) (h : runWindow id socket closed observed panes = .active label) :
    ∃ p ∈ panes, p.id = id ∧ p.socket = socket := by
  unfold runWindow at h
  split at h <;> try contradiction
  split at h <;> try contradiction
  split at h <;> try contradiction
  rename_i pane hf
  refine ⟨pane, List.mem_of_find?_eq_some hf, ?_⟩
  simpa [windowMatches] using List.find?_some hf

theorem unreadable_paneList_cannot_confirm_absence (id socket : Str) (closed : Bool)
    (panes : List RunPane) (ids : Option (List Str))
    (h : paneListObserved ids = false) :
    runWindow id socket closed (paneListObserved ids) panes ≠ .absent := by
  intro absent
  have observed := (runWindow_absent_requires_observation id socket closed _ panes absent).1
  simp [h] at observed

example : runWindow "p".toList "primary".toList false true
    [⟨"p".toList, "secondary".toList, []⟩] = .absent := rfl

example : runWindow "p".toList "primary".toList false false
    [⟨"p".toList, "primary".toList, []⟩] = .unknown := rfl

example : runWindow [] "primary".toList true false [] = .closed := rfl

example : runWindow "p".toList "primary".toList false true
    [⟨"p".toList, "primary".toList, []⟩] = .active "p".toList := rfl

/-! ## Codex 履歴の紐付け

Herdr が現在の pane に報告した ID と、標準配置のファイル名・先頭行から読んだ ID を照合する。
探索の完全性は入力。UUID の構文検査、ディレクトリ探索、JSON の解釈、Herdr 側の生存判定は実装の責任とする。
-/

structure Rollout where
  filenameId : Str
  headerId : Option Str

def selectRollout (complete : Bool) (reported : Option Str) (candidates : List Rollout) : Option Rollout :=
  match complete, reported, candidates with
  | true, some id, [r] =>
    if r.filenameId = id ∧ r.headerId = some id then some r else none
  | _, _, _ => none

theorem selectRollout_sound (complete : Bool) (reported : Option Str) (candidates : List Rollout) (r : Rollout)
    (h : selectRollout complete reported candidates = some r) :
    complete = true ∧ candidates = [r] ∧ reported = some r.filenameId ∧ r.headerId = reported := by
  unfold selectRollout at h
  split at h <;> try contradiction
  split at h <;> try contradiction
  rename_i hverified
  cases h
  exact ⟨rfl, rfl, congrArg some hverified.1.symm, hverified.2⟩

example : selectRollout false (some ['a']) [⟨['a'], some ['a']⟩] = none := rfl
example : selectRollout true none [⟨['a'], some ['a']⟩] = none := rfl
example : selectRollout true (some ['a']) [⟨['a'], some ['b']⟩] = none := rfl
example : selectRollout true (some ['a']) [⟨['a'], some ['a']⟩, ⟨['a'], some ['a']⟩] = none := rfl
example : (selectRollout true (some ['a']) [⟨['a'], some ['a']⟩]).isSome = true := rfl

/-! ## transcript の増分読み取り

同じファイルに完全な行が追記されると仮定する。サイズが読み取り位置を下回るか、
ファイルの識別子が変わったら、それまでの要約を捨てて先頭から読む。
-/

def resumeOffset (sameFile : Bool) (size offset : Nat) : Nat :=
  if sameFile && offset ≤ size then offset else 0

theorem resumeOffset_sameFile (size offset : Nat) (h : offset ≤ size) :
    resumeOffset true size offset = offset := by
  simp [resumeOffset, h]

theorem resumeOffset_shrunk (sameFile : Bool) (size offset : Nat) (h : size < offset) :
    resumeOffset sameFile size offset = 0 := by
  simp [resumeOffset, Nat.not_le_of_lt h]

theorem resumeOffset_replaced (size offset : Nat) : resumeOffset false size offset = 0 := by
  simp [resumeOffset]

/-- 完全な行をまとめて読む場合も、追記だけを読む場合も、同じ順番で要約に反映する。 -/
theorem fold_append {α β : Type} (step : α → β → α) (initial : α) (prefixLines suffix : List β) :
    (prefixLines ++ suffix).foldl step initial = suffix.foldl step (prefixLines.foldl step initial) :=
  List.foldl_append

/-! ## Enter での tab 移動 -/

/-- 一覧取得に使った socket の同一性 (正規化済み) と、実際に渡すパスを分ける。
    `none` のパスは Herdr の暗黙指定。`named` は複数 session の一覧。 -/
def tabTarget (named : Bool) (own listed : Str) (path : Option Str) (tab : Str) :
    Option (Option Str × Str) :=
  if (named && listed != own) || tab.isEmpty then none else some (path, tab)

theorem tabTarget_uses_listed_path (named : Bool) (own listed : Str) (path dest : Option Str) (tab id : Str)
    (h : tabTarget named own listed path tab = some (dest, id)) : dest = path ∧ id = tab := by
  unfold tabTarget at h
  split at h <;> simp_all

theorem tabTarget_rejects_foreign (own listed : Str) (path : Option Str) (tab : Str) (h : listed ≠ own) :
    tabTarget true own listed path tab = none := by
  simp [tabTarget, h]

example : tabTarget false ['s'] ['a'] (some ['p']) ['t'] = some (some ['p'], ['t']) := rfl
example : tabTarget true ['s'] ['s'] (some ['s']) ['t'] = some (some ['s'], ['t']) := rfl
example : tabTarget true ['s'] ['a'] (some ['a']) ['t'] = none := rfl
example : tabTarget false ['s'] ['s'] none ['t'] = some (none, ['t']) := rfl
example : tabTarget false ['s'] ['s'] none [] = none := rfl

end HerdrTimeline
