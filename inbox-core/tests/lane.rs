use inbox_core::*;

fn idx(l: Lane) -> usize {
    match l {
        Lane::Todo => 0,
        Lane::Worker => 1,
        Lane::HumanDecision => 2,
        Lane::HumanAction => 3,
        Lane::Done => 4,
    }
}

const LANES: [Lane; 5] = [
    Lane::Todo,
    Lane::Worker,
    Lane::HumanDecision,
    Lane::HumanAction,
    Lane::Done,
];
const PANES: [Panes; 4] = [Panes::Unknown, Panes::Absent, Panes::Closed, Panes::Live];
const REM: [Remaining; 3] = [Remaining::Empty, Remaining::Nothing, Remaining::Other];

// 全組で next_lane を繰り返し当てる。(a) 3 回以内に None (b) 一度通った lane に戻らない (c) done / human_action は最初から None
#[test]
fn next_lane_は往復せず収束する() {
    let (now, cap) = (100_000u64, 30u64);
    let mut n = 0u64;
    for from in LANES {
        for panes in PANES {
            for bits in 0u32..(1 << 10) {
                let b = |i: u32| bits >> i & 1 == 1;
                for remaining in REM {
                    for detected_at in [now - cap * 60, now - cap * 60 + 1] {
                        let f = TaskFacts {
                            panes,
                            idle: b(0),
                            working: b(1),
                            close_failed: b(2),
                            accepted: b(3),
                            req_accepted: b(4),
                            remaining,
                            waiting: b(5),
                            answered: b(6),
                            detected: b(7),
                            detected_at,
                            seen: b(8),
                            cleared: b(9),
                        };
                        n += 1;
                        let mut cur = from;
                        let mut seen = [false; 5];
                        seen[idx(cur)] = true;
                        let mut steps = 0;
                        while let Some(m) = next_lane(cur, f, now, cap) {
                            steps += 1;
                            cur = m.dest;
                            assert!(
                                !seen[idx(cur)],
                                "戻った: from={} bits={bits} steps={steps}",
                                idx(from)
                            );
                            seen[idx(cur)] = true;
                            assert!(
                                steps <= 3,
                                "3 回で止まらない: from={} bits={bits}",
                                idx(from)
                            );
                        }
                        if matches!(from, Lane::Done | Lane::HumanAction) {
                            assert_eq!(steps, 0);
                        }
                    }
                }
            }
        }
    }
    assert_eq!(n, 5 * 4 * 1024 * 3 * 2);
}

fn sf(idle: bool, report: u64, assign: u64) -> StallFacts {
    StallFacts {
        idle,
        report,
        assign,
    }
}

#[test]
fn stalled_の境界() {
    let (t0, min) = (5000u64, 10u64);
    assert!(
        matches!(stalled(sf(true, t0, 0), t0 + min * 60 - 1, min), Stall::Due(t) if t == t0 + min * 60)
    );
    assert!(matches!(
        stalled(sf(true, t0, 0), t0 + min * 60, min),
        Stall::Stalled(Origin::Report)
    ));
    assert!(matches!(
        stalled(sf(false, t0, 0), t0 + min * 60, min),
        Stall::Active
    ));
    assert!(matches!(stalled(sf(true, 0, 0), 99999, min), Stall::Active));
    assert!(matches!(
        stalled(sf(true, t0, t0), t0 + min * 60, min),
        Stall::Stalled(Origin::Report)
    ));
    assert!(matches!(
        stalled(sf(true, t0 + 1, t0), t0 + 1 + min * 60, min),
        Stall::Stalled(Origin::Report)
    ));
    assert!(matches!(
        stalled(sf(true, t0, t0 + 1), t0 + 1 + min * 60, min),
        Stall::Stalled(Origin::Assign)
    ));
}

fn base(panes: Panes) -> TaskFacts {
    TaskFacts {
        panes,
        idle: false,
        working: false,
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

fn mv(from: Lane, f: TaskFacts) -> Option<(usize, usize)> {
    let (now, cap) = (100_000u64, 30u64);
    next_lane(from, f, now, cap).map(|m| (m.rule as usize, idx(m.dest)))
}

// 収束の試験は規則の枝を消しても通るので、規則ごとの行き先と、条件が重なったときに勝つ規則を当てる
#[test]
fn next_lane_の規則と優先順() {
    use Rule::*;
    let (now, cap) = (100_000u64, 30u64);
    let at = |r: Rule, l: Lane| Some((r as usize, idx(l)));

    // StallCap: 最初の検出からちょうど cap 分で移し、1 秒足りなければ移さない
    let stall = TaskFacts {
        idle: true,
        detected: true,
        seen: true,
        detected_at: now - cap * 60,
        ..base(Panes::Live)
    };
    assert_eq!(mv(Lane::Worker, stall), at(StallCap, Lane::HumanAction));
    assert_eq!(
        mv(
            Lane::Worker,
            TaskFacts {
                detected_at: now - cap * 60 + 1,
                ..stall
            }
        ),
        None
    );
    assert_eq!(
        mv(
            Lane::Worker,
            TaskFacts {
                cleared: true,
                ..stall
            }
        ),
        None
    );

    // PaneGone は worker の pane が無いときだけ。受け入れ済みなら SettleGone が勝つ
    assert_eq!(
        mv(Lane::Worker, base(Panes::Absent)),
        at(PaneGone, Lane::Todo)
    );
    assert_eq!(mv(Lane::Todo, base(Panes::Absent)), None);
    let gone = TaskFacts {
        accepted: true,
        ..base(Panes::Closed)
    };
    assert_eq!(
        mv(
            Lane::Worker,
            TaskFacts {
                panes: Panes::Absent,
                ..gone
            }
        ),
        at(SettleGone, Lane::HumanAction)
    );
    assert_eq!(
        mv(
            Lane::Todo,
            TaskFacts {
                remaining: Remaining::Empty,
                ..gone
            }
        ),
        at(SettleGone, Lane::Done)
    );
    assert_eq!(
        mv(
            Lane::Todo,
            TaskFacts {
                waiting: true,
                ..gone
            }
        ),
        at(SettleGone, Lane::HumanDecision)
    );

    // SettleDone は StallCap の条件が揃っていても勝つ
    let done = TaskFacts {
        req_accepted: true,
        remaining: Remaining::Nothing,
        ..stall
    };
    assert_eq!(mv(Lane::Worker, done), at(SettleDone, Lane::Done));
    assert_eq!(
        mv(
            Lane::Worker,
            TaskFacts {
                close_failed: true,
                ..done
            }
        ),
        at(StallCap, Lane::HumanAction)
    );

    // Waiting は StallCap と Live に勝つ
    assert_eq!(
        mv(
            Lane::Worker,
            TaskFacts {
                waiting: true,
                ..stall
            }
        ),
        at(Waiting, Lane::HumanDecision)
    );
    assert_eq!(
        mv(
            Lane::Todo,
            TaskFacts {
                waiting: true,
                ..base(Panes::Live)
            }
        ),
        at(Waiting, Lane::HumanDecision)
    );

    // Answered は判断待ちが残っていれば移さない
    let ans = TaskFacts {
        answered: true,
        ..base(Panes::Live)
    };
    assert_eq!(mv(Lane::HumanDecision, ans), at(Answered, Lane::Todo));
    assert_eq!(
        mv(
            Lane::HumanDecision,
            TaskFacts {
                waiting: true,
                ..ans
            }
        ),
        None
    );

    // Live: 回答済みで working でない todo は残す
    assert_eq!(mv(Lane::Todo, base(Panes::Live)), at(Live, Lane::Worker));
    assert_eq!(mv(Lane::Todo, ans), None);
    assert_eq!(
        mv(
            Lane::Todo,
            TaskFacts {
                working: true,
                ..ans
            }
        ),
        at(Live, Lane::Worker)
    );
}
