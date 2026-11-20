// worker の差し替え (ADR-322)。lib.rs の next_lane (Lean の証明の対象) の結果を包み、上限 T の 1 回目を human_action でなく差し替えに回す。
// lib.rs は sha256 を固定した証明の対象なので、差し替えの遷移はここに置いて next_lane は触らない。
// 差し替えは lane を動かさない (worker のまま新しい pane に替わる) ので Move でなく別の結果にする。
use crate::*;

// next_lane の事実 (TaskFacts) に足す、差し替えと起きた時刻の事実。時刻は epoch 秒
#[derive(Clone, Copy)]
pub struct StopFacts {
    // 差し替えた時刻 (新しい worker への最初の 1 通の送信)。0 は未差し替え
    pub replaced_at: u64,
    // 起きた時刻 (前の回から 15 分を越えて空いた回)。0 は無し
    pub woke_at: u64,
    // 再送の段を使い切った (resend_step が Exhausted)
    pub exhausted: bool,
}

pub enum Verdict {
    Stay,
    Replace,
    Move(Move),
}

pub fn decide(from: Lane, f: TaskFacts, s: StopFacts, now: u64, cap_min: u64) -> Verdict {
    // 数え直し: 差し替え前の検出は新しい worker の停止でなく、起きた時刻より前の検出は起きた時刻から T を数える
    let f = TaskFacts {
        detected: f.detected && f.detected_at > s.replaced_at,
        detected_at: f.detected_at.max(s.woke_at),
        ..f
    };
    let first = s.replaced_at == 0;
    match next_lane(from, f, now, cap_min) {
        Some(m) if matches!(m.rule, Rule::StallCap) && first => Verdict::Replace,
        Some(m) => Verdict::Move(m),
        None if first
            && s.exhausted
            && f.detected
            && f.seen
            && !f.cleared
            && matches!(from, Lane::Worker)
            && matches!(f.panes, Panes::Live)
            && f.idle =>
        {
            Verdict::Replace
        }
        None => Verdict::Stay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 100_000;
    const CAP: u64 = 30;

    // pane が生きて idle な worker が、d に停止を最初に検出された事実
    fn stalled_at(d: u64) -> TaskFacts {
        TaskFacts {
            panes: Panes::Live,
            idle: true,
            working: false,
            close_failed: false,
            accepted: false,
            req_accepted: false,
            remaining: Remaining::Other,
            waiting: false,
            answered: false,
            detected: true,
            detected_at: d,
            seen: true,
            cleared: false,
        }
    }

    const FRESH: StopFacts = StopFacts {
        replaced_at: 0,
        woke_at: 0,
        exhausted: false,
    };

    fn show(v: Verdict) -> String {
        match v {
            Verdict::Stay => "stay".into(),
            Verdict::Replace => "replace".into(),
            Verdict::Move(m) => format!("move {} {}", m.rule as usize, m.dest as usize),
        }
    }
    const STALL_CAP_HA: &str = "move 5 3"; // StallCap -> HumanAction
    const PANE_GONE_TODO: &str = "move 2 0";

    fn go(from: Lane, f: TaskFacts, s: StopFacts, now: u64) -> String {
        show(decide(from, f, s, now, CAP))
    }

    #[test]
    fn 上限_t_の_1_回目は_human_action_でなく差し替え() {
        let d = NOW - CAP * 60;
        assert_eq!(go(Lane::Worker, stalled_at(d), FRESH, NOW), "replace");
        assert_eq!(go(Lane::Worker, stalled_at(d + 1), FRESH, NOW), "stay");
    }

    #[test]
    fn 差し替え後の_2_回目の停止は_human_action() {
        // 新しい worker は replaced_at の後に検出される。T は最初の検出から数える
        let s = StopFacts {
            replaced_at: 50_000,
            ..FRESH
        };
        let d = 60_000;
        assert_eq!(
            go(Lane::Worker, stalled_at(d), s, d + CAP * 60),
            STALL_CAP_HA
        );
        assert_eq!(go(Lane::Worker, stalled_at(d), s, d + CAP * 60 - 1), "stay");
    }

    #[test]
    fn 再送の段を使い切れば_t_の前でも差し替え() {
        let s = StopFacts {
            exhausted: true,
            ..FRESH
        };
        assert_eq!(go(Lane::Worker, stalled_at(NOW - 60), s, NOW), "replace");
        // 差し替え済みなら段を使い切っても重ねて差し替えない
        let s2 = StopFacts {
            replaced_at: 1,
            ..s
        };
        assert_eq!(go(Lane::Worker, stalled_at(NOW - 60), s2, NOW), "stay");
    }

    #[test]
    fn 差し替え前の検出は数えず_t_を数え直す() {
        // 古い worker の検出 (d <= replaced_at) は新しい worker の停止ではない
        let s = StopFacts {
            replaced_at: 95_000,
            ..FRESH
        };
        let old = NOW - CAP * 60 - 5_000;
        assert_eq!(go(Lane::Worker, stalled_at(old), s, NOW), "stay");
        assert_eq!(
            go(Lane::Worker, stalled_at(95_000), s, NOW + 90_000),
            "stay"
        );
    }

    #[test]
    fn 起きた時刻より前の検出は起きた時刻から_t_を数える() {
        let (d, w) = (1_000u64, 50_000u64);
        let s = StopFacts {
            woke_at: w,
            ..FRESH
        };
        // 検出から数えれば越えている時刻でも、起きた時刻から T 経つまでは移さない
        assert_eq!(go(Lane::Worker, stalled_at(d), s, w + CAP * 60 - 1), "stay");
        assert_eq!(go(Lane::Worker, stalled_at(d), s, w + CAP * 60), "replace");
        // 検出のほうが新しければ起きた時刻は効かない
        let s = StopFacts {
            woke_at: d - 1,
            ..FRESH
        };
        assert_eq!(go(Lane::Worker, stalled_at(d), s, d + CAP * 60), "replace");
    }

    #[test]
    fn 起きた時刻の後の差し替え後の停止も_human_action_まで数え直す() {
        let s = StopFacts {
            replaced_at: 60_000,
            woke_at: 70_000,
            ..FRESH
        };
        let d = 61_000;
        assert_eq!(
            go(Lane::Worker, stalled_at(d), s, 70_000 + CAP * 60 - 1),
            "stay"
        );
        assert_eq!(
            go(Lane::Worker, stalled_at(d), s, 70_000 + CAP * 60),
            STALL_CAP_HA
        );
    }

    #[test]
    fn 差し替えは他の判定に勝たず_worker_の_idle_な_pane_だけ() {
        let s = StopFacts {
            exhausted: true,
            ..FRESH
        };
        let f = stalled_at(NOW - CAP * 60);
        // 判断待ちや pane の消失が先
        assert_eq!(
            go(Lane::Worker, TaskFacts { waiting: true, ..f }, s, NOW),
            "move 4 2"
        );
        assert_eq!(
            go(
                Lane::Worker,
                TaskFacts {
                    panes: Panes::Absent,
                    ..f
                },
                s,
                NOW
            ),
            PANE_GONE_TODO
        );
        // worker 以外、pane が生きていない、idle でないときは差し替えない
        assert_eq!(
            go(
                Lane::Todo,
                TaskFacts {
                    detected: false,
                    ..f
                },
                s,
                NOW
            ),
            "move 6 1"
        );
        assert_eq!(go(Lane::HumanAction, f, s, NOW), "stay");
        assert_eq!(
            go(
                Lane::Worker,
                TaskFacts {
                    panes: Panes::Closed,
                    ..f
                },
                s,
                NOW
            ),
            "stay"
        );
        assert_eq!(
            go(Lane::Worker, TaskFacts { idle: false, ..f }, s, NOW),
            "stay"
        );
        assert_eq!(
            go(
                Lane::Worker,
                TaskFacts {
                    cleared: true,
                    detected: false,
                    ..f
                },
                s,
                NOW
            ),
            "stay"
        );
    }
}
