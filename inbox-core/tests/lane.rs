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
