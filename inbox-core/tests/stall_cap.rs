// lean/InboxCore/StallCap.lean の仕様 (effectiveCap / effectiveDetected / effectiveDetectedAt / stallCapOver / decideSpec)
// と Rust 実装 (replace::decide_for_kind および inbox-core CLI の lane 27 引数・stall-cap・batch) の差分テスト。
use inbox_core::*;
use std::io::Write;
use std::process::{Command, Stdio};

#[path = "../src/replace.rs"]
mod replace;

use replace::{
    decide, decide_for_kind, effective_cap_min, StallKind, StopFacts, Verdict,
    DEFAULT_STARTED_CAP_MIN, DEFAULT_UNCLAIMED_CAP_MIN,
};

// ---- Lean 仕様 (InboxCore/StallCap.lean) の直接参照オラクル ----

fn spec_effective_cap(kind: StallKind, f1_cap: u64, f2_cap: u64) -> u64 {
    match kind {
        StallKind::Unclaimed => f1_cap,
        StallKind::Started => f2_cap,
    }
}

fn spec_effective_detected(detected: bool, detected_at: u64, replaced_at: u64) -> bool {
    detected && (replaced_at == 0 || replaced_at < detected_at)
}

fn spec_effective_detected_at(detected_at: u64, woke_at: u64) -> u64 {
    detected_at.max(woke_at)
}

fn spec_stall_cap_over(
    f: TaskFacts,
    s: StopFacts,
    now: u64,
    kind: StallKind,
    f1_cap: u64,
    f2_cap: u64,
) -> bool {
    spec_effective_detected(f.detected, f.detected_at, s.replaced_at)
        && f.seen
        && !f.cleared
        && spec_effective_detected_at(f.detected_at, s.woke_at)
            + spec_effective_cap(kind, f1_cap, f2_cap) * 60
            <= now
}

fn spec_next_lane(fr: Lane, f: TaskFacts, oc: bool) -> Option<(Rule, Lane)> {
    let tw = matches!(fr, Lane::Todo | Lane::Worker);
    let live = matches!(f.panes, Panes::Live);
    let gone = matches!(f.panes, Panes::Absent | Panes::Closed);
    if tw && gone && f.accepted {
        let dest = match f.remaining {
            Remaining::Other => {
                if f.waiting {
                    Lane::HumanDecision
                } else {
                    Lane::HumanAction
                }
            }
            Remaining::Empty | Remaining::Nothing => Lane::Done,
        };
        Some((Rule::SettleGone, dest))
    } else if tw
        && live
        && f.idle
        && f.req_accepted
        && !f.waiting
        && !f.close_failed
        && matches!(f.remaining, Remaining::Nothing)
    {
        Some((Rule::SettleDone, Lane::Done))
    } else if matches!(fr, Lane::Worker) && matches!(f.panes, Panes::Absent) {
        Some((Rule::PaneGone, Lane::Todo))
    } else if matches!(fr, Lane::HumanDecision) && f.answered && !f.waiting {
        Some((Rule::Answered, Lane::Todo))
    } else if tw && live && f.waiting {
        Some((Rule::Waiting, Lane::HumanDecision))
    } else if matches!(fr, Lane::Worker) && live && f.idle && oc {
        Some((Rule::StallCap, Lane::HumanAction))
    } else if matches!(fr, Lane::Todo) && live && !(f.answered && !f.working) {
        Some((Rule::Live, Lane::Worker))
    } else {
        None
    }
}

fn spec_decide(
    fr: Lane,
    f: TaskFacts,
    s: StopFacts,
    now: u64,
    kind: StallKind,
    f1_cap: u64,
    f2_cap: u64,
) -> String {
    let eff_det = spec_effective_detected(f.detected, f.detected_at, s.replaced_at);
    let oc = spec_stall_cap_over(f, s, now, kind, f1_cap, f2_cap);
    let first = s.replaced_at == 0;
    match spec_next_lane(fr, f, oc) {
        Some((Rule::StallCap, dest)) => {
            if first {
                "replace".into()
            } else {
                format!("move {} {}", rule_str(Rule::StallCap), lane_str(dest))
            }
        }
        Some((r, dest)) => format!("move {} {}", rule_str(r), lane_str(dest)),
        None => {
            if first
                && s.exhausted
                && eff_det
                && f.seen
                && !f.cleared
                && matches!(fr, Lane::Worker)
                && matches!(f.panes, Panes::Live)
                && f.idle
            {
                "replace".into()
            } else {
                "none".into()
            }
        }
    }
}

fn show_verdict(v: Verdict) -> String {
    match v {
        Verdict::Stay => "none".into(),
        Verdict::Replace => "replace".into(),
        Verdict::Move(m) => format!("move {} {}", rule_str(m.rule), lane_str(m.dest)),
    }
}

fn lane_str(l: Lane) -> &'static str {
    match l {
        Lane::Todo => "todo",
        Lane::Worker => "worker",
        Lane::HumanDecision => "human_decision",
        Lane::HumanAction => "human_action",
        Lane::Done => "done",
    }
}

fn panes_str(p: Panes) -> &'static str {
    match p {
        Panes::Unknown => "unknown",
        Panes::Absent => "absent",
        Panes::Closed => "closed",
        Panes::Live => "live",
    }
}

fn rem_str(r: Remaining) -> &'static str {
    match r {
        Remaining::Empty => "empty",
        Remaining::Nothing => "nothing",
        Remaining::Other => "other",
    }
}

fn rule_str(r: Rule) -> &'static str {
    match r {
        Rule::SettleGone => "settle-gone",
        Rule::SettleDone => "settle-done",
        Rule::PaneGone => "pane-gone",
        Rule::Answered => "answered",
        Rule::Waiting => "waiting",
        Rule::StallCap => "stall-cap",
        Rule::Live => "live",
    }
}

fn kind_str(k: StallKind) -> &'static str {
    match k {
        StallKind::Unclaimed => "unclaimed",
        StallKind::Started => "started",
    }
}

fn yn(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

fn run_batch_cli(input: &str) -> Vec<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_inbox-core"))
        .arg("batch")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "batch failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(String::from)
        .collect()
}

fn format_lane_27(
    fr: Lane,
    f: TaskFacts,
    s: StopFacts,
    now: u64,
    legacy_cap: u64,
    cap: replace::StallCap,
) -> String {
    format!(
        "lane {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} unset no no no {} {} {}",
        lane_str(fr),
        panes_str(f.panes),
        yn(f.idle),
        yn(f.working),
        yn(f.close_failed),
        yn(f.accepted),
        yn(f.req_accepted),
        rem_str(f.remaining),
        yn(f.waiting),
        yn(f.answered),
        yn(f.detected),
        f.detected_at,
        yn(f.seen),
        yn(f.cleared),
        s.replaced_at,
        s.woke_at,
        yn(s.exhausted),
        now,
        legacy_cap,
        kind_str(cap.kind),
        cap.f1_cap_min,
        cap.f2_cap_min,
    )
}

#[test]
fn 差分テスト_rust純粋関数がlean仕様と境界・全枝で一致する() {
    let lanes = [
        Lane::Todo,
        Lane::Worker,
        Lane::HumanDecision,
        Lane::HumanAction,
        Lane::Done,
    ];
    let panes_all = [Panes::Unknown, Panes::Absent, Panes::Closed, Panes::Live];
    let rems = [Remaining::Empty, Remaining::Nothing, Remaining::Other];
    let kinds = [StallKind::Unclaimed, StallKind::Started];
    let cap_pairs = [
        (DEFAULT_UNCLAIMED_CAP_MIN, DEFAULT_STARTED_CAP_MIN),
        (30, 30),
        (0, 5),
    ];

    let detected_at = 50_000u64;
    let replaced_cases = [0u64, 49_999, 50_000, 50_001];
    let woke_cases = [0u64, 40_000, 60_000];

    let mut checked = 0u64;
    for &(f1, f2) in &cap_pairs {
        for &kind in &kinds {
            let eff_cap = effective_cap_min(kind, f1, f2);
            assert_eq!(eff_cap, spec_effective_cap(kind, f1, f2));
            for &replaced_at in &replaced_cases {
                for &woke_at in &woke_cases {
                    let t0 = detected_at.max(woke_at);
                    let mut nows = vec![t0 + eff_cap * 60];
                    if t0 + eff_cap * 60 > 0 {
                        nows.push(t0 + eff_cap * 60 - 1);
                    }
                    // F1=3分 と F2=15分 の中間の時刻 (t0 + 180) も必ず調べる
                    nows.push(t0 + f1 * 60);
                    for &now in &nows {
                        for &exhausted in &[false, true] {
                            let s = StopFacts {
                                replaced_at,
                                woke_at,
                                exhausted,
                            };
                            for &fr in &lanes {
                                for &panes in &panes_all {
                                    for bits in 0u32..(1 << 6) {
                                        let b = |i: u32| (bits >> i) & 1 == 1;
                                        let f = TaskFacts {
                                            panes,
                                            idle: b(0),
                                            working: !b(0),
                                            close_failed: false,
                                            accepted: b(1),
                                            req_accepted: b(1),
                                            remaining: rems[(bits as usize) % rems.len()],
                                            waiting: b(2),
                                            answered: b(3),
                                            detected: b(4),
                                            detected_at,
                                            seen: b(5),
                                            cleared: (bits & 1 == 1) && (bits & 16 == 0),
                                        };
                                        let got = show_verdict(decide_for_kind(
                                            fr, f, s, now, kind, f1, f2,
                                        ));
                                        let want = spec_decide(fr, f, s, now, kind, f1, f2);
                                        assert_eq!(
                                            got, want,
                                            "mismatch: kind={kind:?} f1={f1} f2={f2} rep={replaced_at} woke={woke_at} now={now} ex={exhausted}"
                                        );
                                        // 後方互換: f1 == f2 なら単一上限 decide と一致
                                        if f1 == f2 {
                                            assert_eq!(
                                                got,
                                                show_verdict(decide(fr, f, s, now, f1))
                                            );
                                        }
                                        // 2 回目の停止では二度と replace を返さない (no_second_replace)
                                        if replaced_at > 0 {
                                            assert_ne!(got, "replace");
                                        }
                                        // 差し替え済みかつ古い検出は stall-cap も replace も返さない (recount_blocks_stall)
                                        if replaced_at > 0 && detected_at <= replaced_at {
                                            assert_ne!(got, "replace");
                                            assert_ne!(got, "move stall-cap human_action");
                                        }
                                        // sleep 除外 (sleep_excludes_move / sleep_excludes_replace)
                                        if now < woke_at + eff_cap * 60 {
                                            assert_ne!(got, "move stall-cap human_action");
                                            if !exhausted {
                                                assert_ne!(got, "replace");
                                            }
                                        }
                                        checked += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 100_000, "expected >100k cases, got {checked}");
}

#[test]
fn 差分テスト_cliのlane27引数とstall_capがlean仕様および従来形式と一致する() {
    let mut batch_in = String::new();
    let mut expected = Vec::new();

    // 1. stall-cap サブコマンド (既定と明示指定)
    for (cmd, want) in [
        ("stall-cap unclaimed", "3"),
        ("stall-cap started", "15"),
        ("stall-cap unclaimed 2 10", "2"),
        ("stall-cap started 2 10", "10"),
        ("stall-cap unclaimed 30 30", "30"),
        ("stall-cap started 30 30", "30"),
    ] {
        batch_in.push_str(cmd);
        batch_in.push('\n');
        expected.push(want.to_string());
    }

    // 2. 境界ケースを網羅する lane 27 引数 (F1=3分 / F2=15分 / sleep 除外 / 差し替え数え直し / 1回目 vs 2回目 / 後方互換)
    let base_f = TaskFacts {
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
        detected_at: 50_000,
        seen: true,
        cleared: false,
    };

    for &kind in &[StallKind::Unclaimed, StallKind::Started] {
        for &(f1, f2) in &[(3u64, 15u64), (30, 30)] {
            for &replaced_at in &[0u64, 49_999, 50_000, 50_001] {
                for &woke_at in &[0u64, 40_000, 60_000] {
                    let t0 = base_f.detected_at.max(woke_at);
                    for &now in &[
                        t0 + f1 * 60 - 1,
                        t0 + f1 * 60,
                        t0 + f2 * 60 - 1,
                        t0 + f2 * 60,
                    ] {
                        for &exhausted in &[false, true] {
                            let s = StopFacts {
                                replaced_at,
                                woke_at,
                                exhausted,
                            };
                            // 意図的にダミーの cap_min=99 を渡し、27 引数では f1/f2 が使われることを検証
                            let cap = replace::StallCap {
                                kind,
                                f1_cap_min: f1,
                                f2_cap_min: f2,
                            };
                            let line = format_lane_27(Lane::Worker, base_f, s, now, 99, cap);
                            batch_in.push_str(&line);
                            batch_in.push('\n');
                            expected.push(spec_decide(Lane::Worker, base_f, s, now, kind, f1, f2));
                        }
                    }
                }
            }
        }
    }

    // 3. 後方互換性: 20 引数・24 引数 (unset)・27 引数 (f1=f2=30) の一致を CLI で直接比較
    let s_compat = StopFacts {
        replaced_at: 0,
        woke_at: 0,
        exhausted: false,
    };
    for &now in &[50_000 + 30 * 60 - 1, 50_000 + 30 * 60] {
        let want = spec_decide(
            Lane::Worker,
            base_f,
            s_compat,
            now,
            StallKind::Unclaimed,
            30,
            30,
        );
        // 20 引数
        batch_in.push_str(&format!(
            "lane worker live yes no no no no other no no yes 50000 yes no 0 0 no {now} 30\n"
        ));
        expected.push(want.clone());
        // 24 引数
        batch_in.push_str(&format!(
            "lane worker live yes no no no no other no no yes 50000 yes no 0 0 no {now} 30 unset no no no\n"
        ));
        expected.push(want.clone());
        // 27 引数 (unclaimed / started とも 30 30)
        for &k in &[StallKind::Unclaimed, StallKind::Started] {
            batch_in.push_str(&format_lane_27(
                Lane::Worker,
                base_f,
                s_compat,
                now,
                99,
                replace::StallCap {
                    kind: k,
                    f1_cap_min: 30,
                    f2_cap_min: 30,
                },
            ));
            batch_in.push('\n');
            expected.push(want.clone());
        }
    }

    let got = run_batch_cli(&batch_in);
    assert_eq!(got.len(), expected.len());
    for (i, (g, w)) in got.iter().zip(expected.iter()).enumerate() {
        assert_eq!(g, w, "batch line {} mismatch", i + 1);
    }

    // 4. 単体実行 (stall-cap) の正常終了と不正引数の exit 2 を検証
    let ok = Command::new(env!("CARGO_BIN_EXE_inbox-core"))
        .args(["stall-cap", "unclaimed"])
        .output()
        .unwrap();
    assert!(ok.status.success());
    assert_eq!(String::from_utf8(ok.stdout).unwrap().trim(), "3");

    let bad = Command::new(env!("CARGO_BIN_EXE_inbox-core"))
        .args(["stall-cap", "unknown"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
}
