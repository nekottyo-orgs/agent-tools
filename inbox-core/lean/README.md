# inbox-core の証明

`../src/lib.rs` の `step` と `plan` を Charon と Aeneas で Lean に翻訳し、窓口の受け渡しの model (`Handoff/Model.lean`) の Step を精密化することを示す。経緯と、bash と model の食い違いの扱いは、移す前の repo (非公開) の `adr/318-inbox-core-rust.md` にある。

## ファイル

| ファイル | 中身 | 書き手 |
|---|---|---|
| `Handoff/Model.lean`、`Handoff/Post.lean` | 受け渡しの状態機械と、修正後の形 (`Config.post`) での性質 | 手書き (run の lean-handoff から複製) |
| `InboxCore/InboxCore.lean` | `lib.rs` の Lean 訳 | Aeneas の生成物。手で直さない |
| `InboxCore/Proofs.lean`、`InboxCore/Effects.lean` | 生成された `step` / `plan` についての定理と `#print axioms` | 手書き |
| `InboxCore/Stalled.lean` | 停止の検出 `stalled` の Nat の上の仕様と、生成物がそれに一致することの定理 | 手書き |
| `lib.rs.sha256` | 最後に証明が通った `lib.rs` の sha256 | `regen.sh --update` |
| `regen.sh` | 生成、生成物との差の検査、`lake build`、合否の判定 | |

生成物を commit するのは、13KB と小さく、`lib.rs` を変えたときに翻訳の差を review で読めるため。

## 回し直す

```bash
bash inbox-core/lean/regen.sh           # 生成して commit 済みと比べ、lake build と axioms を確かめる
bash inbox-core/lean/regen.sh --update  # 生成物と lib.rs.sha256 を書き換える
```

Aeneas / Charon / Lean の nightly は `AENEAS_SPIKE` (既定 `$HOME/.local/share/aeneas-spike`) の隔離環境を使い、本番の cargo と mise の設定には触れない。Mathlib と Aeneas の Lean ライブラリはそこの取得済みのものを指す。環境が無ければ「未実行」と出して終わる。

CI はこの環境を持たないので、`.github/workflows/inbox-core.yml` が `lib.rs` の sha256 を `lib.rs.sha256` と照合する。`lib.rs` を変えて証明を回し直していなければ、そこで落ちる。

## 証明した範囲

生成された `step` / `plan` についての定理は 15 本、`stalled` についての定理は U64 の演算の補題 4 本を含めて 8 本で、どれも `#print axioms` が `propext` / `Classical.choice` / `Quot.sound` だけを返す (補題 4 本は `[propext, Quot.sound]`)。`Handoff/Model.lean`、`Handoff/Post.lean` の model 側の定理と補題を含めた 47 本 (`regen.sh` の「定理の数」) のどれにも `sorry` は無い (`regen.sh` が判定する)。

- `step_total`、`plan_total`: 生成された `step` と `plan` は失敗しない (`plan` の副作用は 3 個以下)
- `step_refines`: `step` の判定は model の Step の遷移と一致する。タグが model にあるもの (`InModel`) に限る
- `step_moved_e4`: `step` が msg を cur に移すとき、model の E4 (返信必須の msg は返信か許された no-reply を経ないと cur に入らない) を保つ
- `no_reply_refused_any_header`: escalation / request への `ack --no-reply` は、ヘッダの正否によらず断る
- `escalation_reply_moves`、`escalation_ack_refused`、`escalation_reply_reach`、`plan_escalation_reply`: 定理が空虚でない証拠。具体的な escalation は reply で cur に移り、素の ack は断られ、移った状態は model で到達可能で Resolved
- `ack_complete`、`ackNoReply_complete`、`ackSenderGone_complete`、`reply_complete`、`hold_complete`: 逆向き。model の遷移の guard を満たす msg には、Rust も Moved を返し、その像が model の遷移後と一致する
- `other_no_reply_moves`: model に無い語彙外のタグは `--no-reply` で閉じる (E4 はこのタグについて何も言わない)

`stalled` は model の Step でなく、`Stalled.lean` の `stallSpec` (Nat の上で、起点 t0 = max report assign、`!idle ∨ t0 = 0` なら Active、`now < t0 + min*60` なら Due、それ以外は停止で `assign ≤ report` なら Report) に照らす。

- `stalled_refines`: 生成された `stalled` は失敗せず、結果が `stallSpec` と一致する。前提は溢れだけで、`idle` かつ t0 ≠ 0 のとき t0 + min*60 ≤ U64.max
- `stalled_overflow_fails`: その前提が破れると、`min * 60` と `t0 + min*60` のどちらで溢れても `fail .integerOverflow` を返す
- `stalled_complete`: `idle` で 0 < t0 かつ t0 + min*60 ≤ now なら、`stalled` は停止 (`assign ≤ report` なら Report、でなければ Assign) を返す。now が U64 に収まるので溢れの前提は要らない
- `stalled_sound`: `stalled` が停止を返すなら、`idle` で 0 < t0 かつ t0 + min*60 ≤ now で、起点が Report であることと `assign ≤ report` は同値。溢れは fail になるので前提は要らない
- `u64_mul_ok`、`u64_add_ok`、`u64_mul_fail`、`u64_add_fail`: 上の証明が使う、U64 の `*` / `+` が範囲内で ok、範囲外で fail を返すことの等式

この範囲に公平性と有限性の前提は無い (`stalled` は 1 回の判定で、時間の経過を扱わない)。

## 前提の名前

`.lean` のコメントが「README の」と書く前提は次のとおり。名前は run の lean-handoff の README から引き継いだ。

- A5: reply の送信が成功した (herdr-message / agent prompt が exit 0) ことと、送り元に届いたことを同じとみなす
- A6: `--sender-gone` の確認は正しい。pane id が再利用されても誤認しない
- A7: 修正後の張り直しは、new/ の残りを即時に全件出す (`e5_of` が依る遷移の定義)
- A8: watch が止まった後、窓口の張り直し (`Rearm`) はいつか走る。機械では保証せず運用の仮定に置く (`post_watchDie_rearm` の前提)
- G5: model の `Config.stage0` では E5 が成り立つが、移行期の現実はそれより弱い (移す前の repo の `skills/herdr-orchestrate/references/front-desk.md` の張り直しの節)

## 残る限界

- 受信待ちの停止 (issue 41 の穴 2 の残り): `Fault.watchDie` の後は `armed` が落ち、次に届いた msg を watch は通知しない。直したのは fire の guard だけで、この経路は残る。言えるのは `post_watchDie_rearm` の「張り直しが 1 回走れば new/ を全て見せるか受信待ちを張り直す」までで、張り直しが走ることは A8 の仮定
- token ファイルの掃除 (issue 41 の穴 1): `Step.sweep` は「消しうる」だけを表す。model に時間が無いので、実装の lease 期限 (24 時間) が過ぎれば必ず消える、とは言っていない。次の claim が来なければ、孤立したファイルは残り続ける
- `lib.rs` の `next_lane` / `resend_step` (取り込み側の lane の照合、再送の段の判定) は Lean に翻訳されて build に通るが、まだ定理が無い。適合の根拠は `../tests/` の全数の試験だけ

## 証明の外

- `../src/main.rs`: 引数の解析と出力の整形
- run-inbox.sh の外殻: ファイルの読み書き、herdr への問い合わせ、送信、mv、複数 msg を全件移すか 0 件にするか、`core_decide` の出力の形の検査と bash の表への fallback、`cue_ready` の拒否
- 外殻の観測が正しいこと: 送り元の生死 (model の前提 A6)、送信の成功が到達を意味すること (A5)、ヘッダとタグの読み取り
- model に無いタグ (Info / Conflict / Other)。`step_refines` の仮定 `InModel` の外で、返信要否は herdr-message の語彙に従う
- Charon と Aeneas の翻訳が正しいこと、手書きの model が現実の運用に合っていること。後者の根拠は証明でなく、run の lean-handoff の再生と性質検査 (bash と Rust の判定を 3 本の trace で突き合わせた結果)
