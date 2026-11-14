/-
目的: BashGates.lean のモデル関数 (hasWord / triggered / select / merge) に入力を流し、結果を
      vectors.txt に書く。main.rs の #[cfg(test)] がこれを読み、Rust の同名関数と 1 件ずつ比べる。

形式 (why): 1 行 1 件、空白区切り。バイト列は `=` + hex (空なら `=` だけ)、Option の none は `-`。
  文字列のまま書くと quote・改行・多バイト文字の扱いが Lean と Rust のパーサで違いうるので、
  比べたいのは関数だけになるようバイト列で渡す。
  - `h <s> <w> <hasWord>`
  - `t <語の数> <語…> <部分文字列の数> <部分文字列…> <raw> <triggered e [raw]> <select [e] (some raw) が e を選ぶか>`
  - `m <子の数> <子…> <結果>`。子は `-` か `o <name> <decision> <reason> <ctx> <sys>`、
    結果は `-` か `r <decision> <reason> <ctx> <sys>`

使い方: repo root で BashGates.lean の冒頭コメントにあるコマンドを打つ。乱数は固定 seed の LCG なので
  何度作り直しても同じ vectors.txt になる。
  CMDS_JSON (Bash command の JSON 配列) と CMDS_OUT を渡すと、先頭 500 件を sample の entry 全員と
  照合した行を CMDS_OUT にも書く。個人の作業コマンドを repo に残さないため vectors.txt には混ぜない
-/
import BashGates
import Lean.Data.Json

open BashGates

deriving instance Inhabited for Entry

def bytesOf (s : String) : Bytes := s.toUTF8.toList

def hex (b : Bytes) : String :=
  b.foldl (fun acc x => (acc.push (Nat.digitChar (x.toNat / 16))).push (Nat.digitChar (x.toNat % 16))) "="

def optHex : Option String → String
  | none => "-"
  | some s => hex (bytesOf s)

def bit (b : Bool) : String := if b then "1" else "0"

def entry (name : String) (ws ss : List String) : Entry := ⟨name, ws.map bytesOf, ss.map bytesOf⟩

/-- 取り込み側の manifest の dispatched に並ぶ entry を想定した例。名前ときっかけ語は汎用に置き換えてある -/
def sample : List Entry := [
  entry "post-check" ["gh"] [],
  entry "path-guard" [] ["inbox-dir"],
  entry "commit-check" ["git"] ["COMMIT_CHECK_SKIP"],
  entry "scan-check" ["git"] ["SCAN_SKIP"],
  entry "add-check" ["git"] ["ADD_OK"],
  entry "push-check" ["git"] ["PUSH_CHECK_SKIP"],
  entry "eval-check" ["git"] ["EVAL_SKIP"],
  entry "lockfile-check" [] ["pnpm-lock.yaml", "package-lock.json", "npm-shrinkwrap.json",
    "yarn.lock", "bun.lock", "go.sum", "gradle.lockfile"],
  entry "rotate-check" ["svc"] ["rotate-self", "ROTATE_OK"],
  entry "dispatch-check" ["svc"] ["rotate-other", "DISPATCH_OK"],
  entry "cd-check" ["cd"] ["CD_OK"]]

/-- 空の語・空の部分文字列・何も持たない entry -/
def edge : List Entry := [
  entry "empty-word" [""] [],
  entry "empty-sub" [] [""],
  entry "nothing" [] [],
  entry "git-and-empty" ["git", ""] []]

def entries : List Entry := sample ++ edge

def sampleWords : List String := ["gh", "git", "svc", "cd"]
def sampleSubs : List String := (sample.flatMap (·.substrings)).map fun b => String.fromUTF8! ⟨b.toArray⟩

/-- 語の前後に置く文字。語を作る文字・作らない文字・quote・空白・改行・多バイト・端 (空) -/
def ctxs : List String := ["", "_", ".", "-", "a", "Z", "0", "\\", "\"", "'", " ", "\n", "あ", "/", ";", "=", "$("]

def quotes : List String := ["\"", "'", "\\", "\"\""]

/-- 語の途中に quote を挟んだもの (g"it など) -/
def splitQuoted (w : String) : List String :=
  (List.range (w.length - 1)).flatMap fun k =>
    quotes.map fun q => String.ofList (w.toList.take (k + 1)) ++ q ++ String.ofList (w.toList.drop (k + 1))

def hLine (s w : Bytes) : String := s!"h {hex s} {hex w} {bit (hasWord s w)}"

def tLine (e : Entry) (raw : String) : String :=
  let r := bytesOf raw
  " ".intercalate (["t", toString e.words.length] ++ e.words.map hex ++
    [toString e.substrings.length] ++ e.substrings.map hex ++
    [hex r, bit (triggered e [r]), bit ((select [e] (some r)).length == 1)])

def oTokens : Option Outcome → List String
  | none => ["-"]
  | some o => ["o", hex (bytesOf o.name), optHex o.decision, hex (bytesOf o.reason), optHex o.ctx, optHex o.sys]

def mLine (os : List (Option Outcome)) : String :=
  let res := match merge os with
    | none => ["-"]
    | some o => ["r", optHex o.decision, optHex o.reason, hex (bytesOf o.ctx), hex (bytesOf o.sys)]
  " ".intercalate (["m", toString os.length] ++ os.flatMap oTokens ++ res)

def lcg (s : Nat) : Nat := (s * 1103515245 + 12345) % 2147483648

/-- 乱数で組む文字列の部品。sample の語の文字を多めに入れて、偶然の一致も起きるようにする -/
def alphabet : List String :=
  ["g", "i", "t", "h", "c", "d", "e", "r", "a", "s", " ", " ", "\n", "_", ".", "-", "\\", "\"", "'",
   "/", "あ", ";", "|", "0", "S", "K", "P", "O", "git", "gh", "cd", "svc", "_SKIP", "lock"]

def vectors : List String := Id.run do
  let mut out : Array String := #[]
  -- has_word: 語を前後の文字で挟む (行頭・行末は空の ctx)
  for w in sampleWords ++ ["", "x"] do
    for p in ctxs do
      for q in ctxs do
        out := out.push (hLine (bytesOf (p ++ w ++ q)) (bytesOf w))
  for w in sampleWords do
    for s in splitQuoted w do
      out := out.push (hLine (bytesOf ("x " ++ s)) (bytesOf w))
      out := out.push (hLine (bytesOf s) (bytesOf w))
  -- triggered / select: きっかけ語を前後の文字で挟み、持ち主の entry・別の entry・端の entry で照合する
  let toks := sampleWords ++ sampleSubs
  let mut k := 0
  for tok in toks do
    for p in ctxs do
      k := k + 1
      let raw := p ++ tok ++ ctxs[(k * 7) % ctxs.length]!
      for e in entries do
        if e.words.any (· == bytesOf tok) || e.substrings.any (· == bytesOf tok) then
          out := out.push (tLine e raw)
      out := out.push (tLine entries[k % entries.length]! raw)
    for s in splitQuoted tok |>.take 8 do
      k := k + 1
      for e in entries do
        if e.words.any (· == bytesOf tok) || e.substrings.any (· == bytesOf tok) then
          out := out.push (tLine e ("echo " ++ s))
      out := out.push (tLine entries[k % entries.length]! s)
  -- 乱数
  let mut rng := 42
  for _ in [0:600] do
    rng := lcg rng
    let n := (rng / 65536) % 12
    let mut s := ""
    for _ in [0:n] do
      rng := lcg rng
      s := s ++ alphabet[(rng / 65536) % alphabet.length]!
    rng := lcg rng
    let wn := (rng / 65536) % 3
    let mut w := ""
    for _ in [0:wn] do
      rng := lcg rng
      w := w ++ alphabet[(rng / 65536) % alphabet.length]!
    out := out.push (hLine (bytesOf s) (bytesOf w))
    rng := lcg rng
    out := out.push (tLine entries[(rng / 65536) % entries.length]! s)
  -- has_word は任意のバイト列も受けるので、UTF-8 として正しくない列も混ぜる
  for _ in [0:150] do
    rng := lcg rng
    let n := (rng / 65536) % 10
    let mut s : Bytes := []
    for _ in [0:n] do
      rng := lcg rng
      s := s ++ [((rng / 65536) % 256).toUInt8]
    rng := lcg rng
    let w := s.drop ((rng / 65536) % 4) |>.take ((rng / 1024) % 3)
    out := out.push (hLine s w)
  -- merge: 理由文を少数から選んで重複 (dedupe) を起こし、ctx / sys には none と空文字の両方を入れる
  let names := ["a", "b", "child-a", "あ"]
  let decisions : List (Option String) := [none, some "deny", some "ask", some "allow", some "Deny"]
  let reasons := ["", "理由A", "r1", "r2", "line\nbreak", "[a]\nr1"]
  let extras : List (Option String) := [none, none, some "", some "c1", some "文脈\n2"]
  for _ in [0:350] do
    rng := lcg rng
    let n := (rng / 65536) % 6
    let mut os : List (Option Outcome) := []
    for _ in [0:n] do
      rng := lcg rng
      let r := rng / 65536
      if r % 5 == 0 then
        os := os ++ [none]
      else
        os := os ++ [some ⟨names[(r / 5) % names.length]!, decisions[(r / 20) % decisions.length]!,
          reasons[(r / 100) % reasons.length]!, extras[(r / 600) % extras.length]!,
          extras[(r / 3000) % extras.length]!⟩]
    out := out.push (mLine os)
  return out.toList

def writeLines (path : String) (ls : List String) : IO Unit :=
  IO.FS.writeFile path (String.join (ls.map (· ++ "\n")))

def main : IO Unit := do
  writeLines "bash-gates/lean/vectors.txt" vectors
  -- CMDS_OUT に既定値を置かない: 既定で cwd (repo root) に書くと実コマンドが repo に紛れ込む
  if let (some cmds, some dest) := (← IO.getEnv "CMDS_JSON", ← IO.getEnv "CMDS_OUT") then
    let j ← IO.ofExcept (Lean.Json.parse (← IO.FS.readFile cmds))
    let arr ← IO.ofExcept j.getArr?
    let cs := (arr.toList.take 500).filterMap fun c => c.getStr?.toOption
    writeLines dest (cs.flatMap fun c => sample.map (tLine · c))
