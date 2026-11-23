// 依頼の種類 kind と「終わりの印」で done を決める (nekottyo/agents issue 408 の段 2)。replace::decide の結果を包む。
// lib.rs の Rule は Lean の証明の対象で足せないので、印による done と merge 待ちは Verdict の別 variant にする。
use crate::replace::{self, StopFacts};
use crate::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    // 種類が空の既存の依頼。従来どおり replace::decide のまま
    Unset,
    Decision,
    Monitor,
    Research,
    Implement,
}

// 外殻が渡す、種類と終わりの印の事実
#[derive(Clone, Copy)]
pub struct KindFacts {
    pub kind: Kind,
    // その kind の終わりの印が成り立つ (implement は PR が merge 済み。他は外殻が判定する)
    pub marker: bool,
    // PR の CI が通っている
    pub ci_green: bool,
    // PR の repo が自分の org でない
    pub other_org: bool,
}

pub enum Verdict {
    Stay,
    Replace,
    Move(Move),
    // 終わりの印が成り立った。dest は done
    EndMarker,
    // merge を人に待つ。dest は human_action
    MergeWait,
}

pub(super) fn wrap(v: replace::Verdict) -> Verdict {
    match v {
        replace::Verdict::Stay => Verdict::Stay,
        replace::Verdict::Replace => Verdict::Replace,
        replace::Verdict::Move(m) => Verdict::Move(m),
    }
}

pub fn judge(
    from: Lane,
    f: TaskFacts,
    s: StopFacts,
    now: u64,
    cap_min: u64,
    k: KindFacts,
) -> Verdict {
    if k.kind == Kind::Unset {
        return wrap(replace::decide(from, f, s, now, cap_min));
    }
    if matches!(from, Lane::Done) {
        return Verdict::Stay;
    }
    if k.marker {
        return Verdict::EndMarker;
    }
    // human_action に入った後に判断待ちが付いたら human_decision へ戻す。human_action からは next_lane が動かさず、問いが人に出ないため。
    // 入った経路 (merge-wait / StallCap) や今の CI には依らず、生きた worker が判断を待つ事実だけで決める。judge に履歴の入力を足さないため
    if matches!(from, Lane::HumanAction) && f.waiting && matches!(f.panes, Panes::Live) {
        return Verdict::Move(Move {
            rule: Rule::Waiting,
            dest: Lane::HumanDecision,
        });
    }
    let before_human_action = matches!(from, Lane::Todo | Lane::Worker | Lane::HumanDecision);
    // 人の判断待ちか、回答を todo へ戻す途中の依頼は merge 待ちで human_action に固定しない。human_action からは next_lane が動かさず、判断の問いが埋もれるため
    // answered は decision_id が task に残り続けるので常に立ちうる。止めるのは回答を todo へ戻す human_decision からだけ
    let answer_pending = matches!(from, Lane::HumanDecision) && f.answered;
    if k.kind == Kind::Implement && k.other_org && k.ci_green && !f.waiting && !answer_pending {
        return if before_human_action {
            Verdict::MergeWait
        } else {
            Verdict::Stay
        };
    }
    match replace::decide(from, f, s, now, cap_min) {
        // 印が無いので報告では done にしない。報告の事実を落として、後続 (pane 無し等) の判定を生かす
        replace::Verdict::Move(m) if matches!(m.dest, Lane::Done) => {
            let f = TaskFacts {
                accepted: false,
                req_accepted: false,
                ..f
            };
            // 報告済みで merge を待つ worker は、報告を落とすと停止に見える。差し替えと停止上限は報告を前提に出る判定なので生かさない
            match replace::decide(from, f, s, now, cap_min) {
                replace::Verdict::Replace => Verdict::Stay,
                replace::Verdict::Move(m) if matches!(m.rule, Rule::StallCap) => Verdict::Stay,
                v => wrap(v),
            }
        }
        v => wrap(v),
    }
}

pub fn judge_for_kind(
    from: Lane,
    f: TaskFacts,
    s: StopFacts,
    now: u64,
    cap: replace::StallCap,
    k: KindFacts,
) -> Verdict {
    if k.kind == Kind::Unset {
        return wrap(replace::decide_for_kind(
            from,
            f,
            s,
            now,
            cap.kind,
            cap.f1_cap_min,
            cap.f2_cap_min,
        ));
    }
    judge(
        from,
        f,
        s,
        now,
        replace::effective_cap_min(cap.kind, cap.f1_cap_min, cap.f2_cap_min),
        k,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 100_000;
    const CAP: u64 = 30;
    const FRESH: StopFacts = StopFacts {
        replaced_at: 0,
        woke_at: 0,
        exhausted: false,
    };

    // pane が生きて working な worker (replace::decide は Live へ動かさず Stay)
    fn base() -> TaskFacts {
        TaskFacts {
            panes: Panes::Live,
            idle: false,
            working: true,
            close_failed: false,
            accepted: false,
            req_accepted: false,
            remaining: Remaining::Other,
            waiting: false,
            answered: false,
            detected: false,
            detected_at: 0,
            seen: false,
            cleared: false,
        }
    }
    // 完了の報告が受け入れ済みで pane が閉じている (decide は SettleGone で done 以外へ)
    fn reported() -> TaskFacts {
        TaskFacts {
            panes: Panes::Absent,
            accepted: true,
            req_accepted: true,
            remaining: Remaining::Nothing,
            ..base()
        }
    }
    // 完了の報告が受け入れ済みで pane は生きて idle (decide は SettleDone で done)
    fn reported_live() -> TaskFacts {
        TaskFacts {
            idle: true,
            working: false,
            accepted: true,
            req_accepted: true,
            remaining: Remaining::Nothing,
            ..base()
        }
    }
    fn kf(kind: Kind) -> KindFacts {
        KindFacts {
            kind,
            marker: false,
            ci_green: false,
            other_org: false,
        }
    }

    fn show(v: Verdict) -> String {
        match v {
            Verdict::Stay => "stay".into(),
            Verdict::Replace => "replace".into(),
            Verdict::Move(m) => format!("move {} {}", m.rule as usize, m.dest as usize),
            Verdict::EndMarker => "end-marker".into(),
            Verdict::MergeWait => "merge-wait".into(),
        }
    }
    fn go(from: Lane, f: TaskFacts, k: KindFacts) -> String {
        show(judge(from, f, FRESH, NOW, CAP, k))
    }
    fn plain(from: Lane, f: TaskFacts) -> String {
        show(wrap(replace::decide(from, f, FRESH, NOW, CAP)))
    }

    const SETTLE_GONE_DONE: &str = "move 0 4"; // SettleGone -> Done
    const PANE_GONE_TODO: &str = "move 2 0";

    #[test]
    fn kind_unset_は_replace_decide_と同一() {
        let lanes = [
            Lane::Todo,
            Lane::Worker,
            Lane::HumanDecision,
            Lane::HumanAction,
            Lane::Done,
        ];
        for from in lanes {
            for f in [base(), reported(), reported_live()] {
                // unset では marker 等が立っていても読まない
                let k = KindFacts {
                    marker: true,
                    ci_green: true,
                    other_org: true,
                    ..kf(Kind::Unset)
                };
                assert_eq!(go(from, f, k), plain(from, f));
            }
        }
    }

    #[test]
    fn done_は戻さず_stay() {
        let k = KindFacts {
            marker: true,
            ..kf(Kind::Implement)
        };
        assert_eq!(go(Lane::Done, base(), k), "stay");
        assert_eq!(go(Lane::Done, reported(), kf(Kind::Monitor)), "stay");
    }

    #[test]
    fn marker_が立てばどの_lane_からでも_end_marker() {
        for kind in [
            Kind::Decision,
            Kind::Monitor,
            Kind::Research,
            Kind::Implement,
        ] {
            let k = KindFacts {
                marker: true,
                ..kf(kind)
            };
            for from in [
                Lane::Todo,
                Lane::Worker,
                Lane::HumanDecision,
                Lane::HumanAction,
            ] {
                assert_eq!(go(from, base(), k), "end-marker");
            }
        }
    }

    #[test]
    fn human_action_の_merge_済みは_end_marker_で報告の有無によらない() {
        let k = KindFacts {
            marker: true,
            ci_green: true,
            other_org: true,
            ..kf(Kind::Implement)
        };
        assert_eq!(go(Lane::HumanAction, base(), k), "end-marker");
        assert_eq!(go(Lane::HumanAction, reported(), k), "end-marker");
    }

    #[test]
    fn 他_org_で_ci_が通った_implement_は_merge_wait() {
        let k = KindFacts {
            ci_green: true,
            other_org: true,
            ..kf(Kind::Implement)
        };
        for from in [Lane::Todo, Lane::Worker, Lane::HumanDecision] {
            assert_eq!(go(from, base(), k), "merge-wait");
        }
        // human_action には重ねて出さない
        assert_eq!(go(Lane::HumanAction, base(), k), "stay");
    }

    #[test]
    fn 判断待ちの_implement_は_merge_wait_で_human_action_に固定しない() {
        let k = KindFacts {
            ci_green: true,
            other_org: true,
            ..kf(Kind::Implement)
        };
        let waiting = TaskFacts {
            waiting: true,
            ..base()
        };
        for from in [Lane::Todo, Lane::Worker, Lane::HumanDecision] {
            assert_ne!(go(from, waiting, k), "merge-wait");
            assert_eq!(go(from, waiting, k), plain(from, waiting));
        }
    }

    #[test]
    fn human_action_の_implement_の判断待ちは_経路と_ci_によらず_human_decision_へ戻る() {
        let k = KindFacts {
            ci_green: true,
            other_org: true,
            ..kf(Kind::Implement)
        };
        let waiting = TaskFacts {
            waiting: true,
            ..base()
        };
        assert_eq!(go(Lane::HumanAction, waiting, k), "move 4 2"); // Waiting -> HumanDecision
                                                                   // CI が赤でも、他 org でなくても (StallCap 経路) 戻る
        for open in [
            KindFacts {
                ci_green: false,
                ..k
            },
            KindFacts {
                other_org: false,
                ..k
            },
            kf(Kind::Implement),
        ] {
            assert_eq!(go(Lane::HumanAction, waiting, open), "move 4 2");
        }
        // 条件が欠ければ動かさない: pane が無い、waiting でない
        let gone = TaskFacts {
            panes: Panes::Absent,
            ..waiting
        };
        assert_eq!(go(Lane::HumanAction, gone, k), "stay");
        let calm = base();
        assert_eq!(go(Lane::HumanAction, calm, k), "stay");
        // 判断待ちが続く間は human_decision に留まる
        assert_eq!(go(Lane::HumanDecision, waiting, k), "stay");
        // 回答後は todo へ戻り、todo から merge-wait で human_action に進む。waiting が止んだ後は human_action で動かない
        let answered = TaskFacts {
            answered: true,
            ..base()
        };
        assert_eq!(go(Lane::HumanDecision, answered, k), "move 3 0"); // Answered -> Todo
        assert_eq!(go(Lane::Todo, answered, k), "merge-wait");
        assert_eq!(go(Lane::HumanAction, answered, k), "stay");
    }

    #[test]
    fn 非実装の_human_action_も生きた_pane_の判断待ちへ移る() {
        let waiting = TaskFacts {
            waiting: true,
            ..base()
        };
        // answered は以前の linked Cue を指すことがある。次の未回答 Cue は waiting で分かる。
        let new_wait = TaskFacts {
            answered: true,
            ..waiting
        };
        let answered = TaskFacts {
            waiting: false,
            ..new_wait
        };
        for kind in [Kind::Decision, Kind::Monitor, Kind::Research] {
            let k = kf(kind);
            assert_eq!(go(Lane::HumanAction, waiting, k), "move 4 2");
            assert_eq!(go(Lane::HumanAction, new_wait, k), "move 4 2");
            assert_eq!(go(Lane::HumanAction, answered, k), "stay");
            for panes in [Panes::Absent, Panes::Closed, Panes::Unknown] {
                assert_eq!(
                    go(Lane::HumanAction, TaskFacts { panes, ..waiting }, k),
                    "stay"
                );
            }
            let marked = KindFacts { marker: true, ..k };
            assert_eq!(go(Lane::HumanAction, waiting, marked), "end-marker");
            assert_eq!(go(Lane::Done, waiting, marked), "stay");
        }
        assert_eq!(go(Lane::HumanAction, waiting, kf(Kind::Unset)), "stay");
    }

    #[test]
    fn 上限前でも停止検出が尽きた報告済みの_implement_は_replace_しない() {
        let f = TaskFacts {
            idle: true,
            working: false,
            req_accepted: true,
            remaining: Remaining::Nothing,
            detected: true,
            detected_at: NOW,
            seen: true,
            ..base()
        };
        let ex = StopFacts {
            exhausted: true,
            ..FRESH
        };
        assert_eq!(
            show(judge(Lane::Worker, f, ex, NOW, CAP, kf(Kind::Implement))),
            "stay"
        );
    }

    #[test]
    fn working_でない回答済みの_todo_も_merge_wait_になる() {
        // decision_id が残り続けるので古い回答と新しい返事待ちを区別できない。段 3 が ci_green を報告済みか ready に絞る前提
        let k = KindFacts {
            ci_green: true,
            other_org: true,
            ..kf(Kind::Implement)
        };
        let idle_answered = TaskFacts {
            answered: true,
            working: false,
            idle: true,
            ..base()
        };
        assert_eq!(go(Lane::Todo, idle_answered, k), "merge-wait");
    }

    #[test]
    fn 回答済みでも_human_decision_以外なら_merge_wait_が出る() {
        // decision_id は task に残り続けるので answered は一度立つと戻らない
        let k = KindFacts {
            ci_green: true,
            other_org: true,
            ..kf(Kind::Implement)
        };
        let answered = TaskFacts {
            answered: true,
            ..base()
        };
        assert_eq!(go(Lane::Todo, answered, k), "merge-wait");
        assert_eq!(go(Lane::Worker, answered, k), "merge-wait");
        // 回答を todo へ戻す human_decision からは従来の判定
        assert_eq!(
            go(Lane::HumanDecision, answered, k),
            plain(Lane::HumanDecision, answered)
        );
        assert_ne!(go(Lane::HumanDecision, answered, k), "merge-wait");
    }

    #[test]
    fn 報告済みで停止検出のある_implement_の_worker_は_replace_も_stall_cap_もしない() {
        // live かつ idle、報告受け入れ済み、最初の停止検出が上限を超えている
        let f = TaskFacts {
            idle: true,
            working: false,
            req_accepted: true,
            remaining: Remaining::Nothing,
            detected: true,
            detected_at: 1000,
            seen: true,
            ..base()
        };
        assert_eq!(
            plain(Lane::Worker, f),
            "move 1 4",
            "前提: 報告だけなら従来は SettleDone"
        );
        assert_eq!(go(Lane::Worker, f, kf(Kind::Implement)), "stay");
        // 差し替え済み (2 回目) の StallCap move も同じく stay
        let again = StopFacts {
            replaced_at: 500,
            ..FRESH
        };
        let f2 = TaskFacts {
            detected_at: 1000,
            ..f
        };
        assert_eq!(
            show(judge(
                Lane::Worker,
                f2,
                again,
                NOW,
                CAP,
                kf(Kind::Implement)
            )),
            "stay"
        );
    }

    #[test]
    fn merge_wait_の条件が欠ければ_merge_wait_にならない() {
        // 自分の org
        let own = KindFacts {
            ci_green: true,
            ..kf(Kind::Implement)
        };
        assert_eq!(go(Lane::Worker, base(), own), plain(Lane::Worker, base()));
        // CI が通っていない
        let red = KindFacts {
            other_org: true,
            ..kf(Kind::Implement)
        };
        assert_eq!(go(Lane::Worker, base(), red), plain(Lane::Worker, base()));
        // implement 以外
        let mon = KindFacts {
            ci_green: true,
            other_org: true,
            ..kf(Kind::Monitor)
        };
        assert_eq!(go(Lane::Worker, base(), mon), plain(Lane::Worker, base()));
    }

    #[test]
    fn marker_無しの報告による_done_は抑えられ呼び直しの結果になる() {
        // 閉じた pane の報告 (SettleGone): 呼び直すと pane 無しの判定が生きる
        assert_eq!(
            plain(Lane::Worker, reported()),
            SETTLE_GONE_DONE,
            "前提: 報告だけなら従来は SettleGone"
        );
        assert_eq!(
            go(Lane::Worker, reported(), kf(Kind::Implement)),
            PANE_GONE_TODO
        );
        // 生きた idle pane の報告 (SettleDone): 呼び直すと後続が無く stay
        assert_eq!(plain(Lane::Worker, reported_live()), "move 1 4"); // SettleDone -> Done
        assert_eq!(
            go(Lane::Worker, reported_live(), kf(Kind::Implement)),
            "stay"
        );
    }

    #[test]
    fn monitor_research_decision_は_marker_無しで報告があっても_done_にならない() {
        for kind in [Kind::Monitor, Kind::Research, Kind::Decision] {
            for f in [reported(), reported_live()] {
                let r = go(Lane::Worker, f, kf(kind));
                assert!(r == "stay" || r == PANE_GONE_TODO, "{kind:?}: {r}");
                assert_ne!(r, "move 0 4");
                assert_ne!(r, "move 1 4");
            }
        }
    }

    #[test]
    fn 報告が残りありで_done_へ動かないなら抑えず従来どおり() {
        // SettleGone でも remaining が other なら human_action へ動く (done ではない)
        let f = TaskFacts {
            remaining: Remaining::Other,
            ..reported()
        };
        assert_eq!(plain(Lane::Worker, f), "move 0 3");
        assert_eq!(go(Lane::Worker, f, kf(Kind::Implement)), "move 0 3");
    }

    #[test]
    fn done_へ動かない判定は従来どおり通る() {
        // 判断待ちは Move (waiting) のまま
        let f = TaskFacts {
            waiting: true,
            ..base()
        };
        assert_eq!(
            go(Lane::Worker, f, kf(Kind::Research)),
            plain(Lane::Worker, f)
        );
    }

    #[test]
    fn judge_for_kind_は停止種別ごとの上限を適用し_unset_と一致する() {
        let d = 50_000u64;
        let f = TaskFacts {
            idle: true,
            working: false,
            detected: true,
            detected_at: d,
            seen: true,
            ..base()
        };
        let cap = |kind| replace::StallCap {
            kind,
            f1_cap_min: 3,
            f2_cap_min: 15,
        };
        // F1=3 分で 180 秒後に replace、F2=15 分は 180 秒では stay・900 秒で replace
        assert_eq!(
            show(judge_for_kind(
                Lane::Worker,
                f,
                FRESH,
                d + 180,
                cap(replace::StallKind::Unclaimed),
                kf(Kind::Implement)
            )),
            "replace"
        );
        assert_eq!(
            show(judge_for_kind(
                Lane::Worker,
                f,
                FRESH,
                d + 180,
                cap(replace::StallKind::Started),
                kf(Kind::Implement)
            )),
            "stay"
        );
        assert_eq!(
            show(judge_for_kind(
                Lane::Worker,
                f,
                FRESH,
                d + 900,
                cap(replace::StallKind::Started),
                kf(Kind::Implement)
            )),
            "replace"
        );
        // Unset は decide_for_kind と完全一致
        for kind in [replace::StallKind::Unclaimed, replace::StallKind::Started] {
            for now in [d + 179, d + 180, d + 899, d + 900] {
                assert_eq!(
                    show(judge_for_kind(
                        Lane::Worker,
                        f,
                        FRESH,
                        now,
                        cap(kind),
                        kf(Kind::Unset)
                    )),
                    show(wrap(replace::decide_for_kind(
                        Lane::Worker,
                        f,
                        FRESH,
                        now,
                        kind,
                        3,
                        15
                    )))
                );
            }
        }
        // 報告済み (SettleDone 相当) の implement は F1 上限を越えても replace / stall-cap しない
        let reported_f = TaskFacts {
            req_accepted: true,
            remaining: Remaining::Nothing,
            ..f
        };
        assert_eq!(
            show(judge_for_kind(
                Lane::Worker,
                reported_f,
                FRESH,
                d + 180,
                cap(replace::StallKind::Unclaimed),
                kf(Kind::Implement)
            )),
            "stay"
        );
    }
}
