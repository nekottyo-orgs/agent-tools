// inbox-core の CLI。run-inbox.sh が msg 1 通ごとに観測を引数で渡し、判定を stdout の 1 行目で受け取る。
// 2 行目以降は判定から決まる副作用の列 (plan)。拒否も判定の 1 つなので exit 0 で返し、
// exit 2 は引数の誤りだけに使う。run-inbox.sh は exit 0 以外と形の崩れた出力を「異常」とみなして bash の判定へ戻る。
use inbox_core::*;
use std::process::exit;

mod replace;

const USAGE: &str = "使い方: inbox-core <op> <tag> <box> <sender> <header> <sent>
  op:     ack | ack-no-reply | ack-sender-gone | reply | hold
  tag:    msg 名から切り出したタグ (語彙外は返信必須として扱う)
  box:    new | hold | cur
  sender: alive | gone | unreadable (ack-sender-gone 以外は読まない)
  header: valid | invalid (ack-sender-gone と reply だけが読む)
  sent:   yes | no (reply だけが読む)
出力: 1 行目 moved <box> <res> | unchanged | refused <理由>、2 行目以降 effect <副作用>
  inbox-core lane <from> <panes> <idle> <working> <close_failed> <accepted> <req_accepted> <remaining> <waiting> <answered> <detected> <detected_at> <seen> <cleared> <replaced_at> <woke_at> <exhausted> <now> <cap_min>
    replaced_at: 差し替えた時刻 (0 は未差し替え)  woke_at: 起きた時刻 (0 は無し)  exhausted: 再送の段を使い切った (bool)
    出力: none | replace | move <rule> <dest>
    from: todo|worker|human_decision|human_action|done  panes: unknown|absent|closed|live  remaining: empty|nothing|other  bool: yes|no  数値: 10 進 (2^40 未満)
  inbox-core stalled <idle> <report> <assign> <now> <min>
    出力: active | due <時刻> | stalled report | stalled assign
  inbox-core resend <tried> <last_nudge> <nudge_sent> <blocked> <agent> <rc> <input> <own> <pasted> <delivered> <overdue>
    input: empty|draft|unknown  出力: 1a | 1b:<理由> | nudge | block:<理由> | exhausted | none
  inbox-core batch   (stdin の各行が lane / stalled / resend の引数列。誤りの行番号を stderr に出して exit 2)";

fn tag(s: &str) -> Tag {
    match s {
        "escalation" => Tag::Escalation,
        "request" => Tag::Request,
        "alert" => Tag::Alert,
        "completion" => Tag::Completion,
        "progress" => Tag::Progress,
        "reply" => Tag::Reply,
        "info" => Tag::Info,
        "conflict" => Tag::Conflict,
        _ => Tag::Other,
    }
}

fn place_name(p: Place) -> &'static str {
    match p {
        Place::New => "new",
        Place::Held => "hold",
        Place::Cur => "cur",
    }
}

fn res_name(r: Res) -> &'static str {
    match r {
        Res::None => "none",
        Res::Replied => "replied",
        Res::NoReply => "no-reply",
        Res::SenderGone => "sender-gone",
    }
}

fn refusal_name(r: Refusal) -> &'static str {
    match r {
        Refusal::NeedsReply => "needs-reply",
        Refusal::SenderAlive => "sender-alive",
        Refusal::SenderUnknown => "sender-unknown",
        Refusal::BadHeader => "bad-header",
        Refusal::InCur => "in-cur",
        Refusal::SendFailed => "send-failed",
    }
}

fn effect_name(e: &Effect) -> String {
    match e {
        Effect::AppendLog(Log::NoReply) => "append-log no-reply".into(),
        Effect::AppendLog(Log::Held) => "append-log held".into(),
        Effect::AppendLog(Log::Replied) => "append-log replied".into(),
        Effect::MoveTo(p) => format!("move-to {}", place_name(*p)),
        Effect::RemoveWait => "remove-wait".into(),
        Effect::WriteWait => "write-wait".into(),
    }
}

fn parse(a: &[String]) -> Option<(Msg, Op, Obs)> {
    if a.len() != 6 {
        return None;
    }
    let op = match a[0].as_str() {
        "ack" => Op::Ack,
        "ack-no-reply" => Op::AckNoReply,
        "ack-sender-gone" => Op::AckSenderGone,
        "reply" => Op::Reply,
        "hold" => Op::Hold,
        _ => return None,
    };
    let place = match a[2].as_str() {
        "new" => Place::New,
        "hold" => Place::Held,
        "cur" => Place::Cur,
        _ => return None,
    };
    let sender = match a[3].as_str() {
        "alive" => Sender::Alive,
        "gone" => Sender::Gone,
        "unreadable" => Sender::Unreadable,
        _ => return None,
    };
    let header_valid = match a[4].as_str() {
        "valid" => true,
        "invalid" => false,
        _ => return None,
    };
    let sent = match a[5].as_str() {
        "yes" => true,
        "no" => false,
        _ => return None,
    };
    // sender (pane id) は判定に使わないので 0 で埋める
    let m = Msg {
        tag: tag(&a[1]),
        sender: 0,
        place,
        res: Res::None,
    };
    Some((
        m,
        op,
        Obs {
            sender,
            sent,
            header_valid,
        },
    ))
}

fn render(m: Msg, op: Op, o: Obs) -> String {
    let head = match step(m, op, o) {
        Outcome::Moved(n) => format!("moved {} {}", place_name(n.place), res_name(n.res)),
        Outcome::Unchanged => "unchanged".into(),
        Outcome::Refused(r) => format!("refused {}", refusal_name(r)),
    };
    let mut out = head;
    for e in plan(m, op, o).iter() {
        out.push_str("\neffect ");
        out.push_str(&effect_name(e));
    }
    out
}

fn yn(s: &str) -> Option<bool> {
    match s {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

// 10 進の数字だけ。2^40 未満 (時刻と分の加算が overflow しないため)
fn num(s: &str) -> Option<u64> {
    if s.is_empty() || s.len() > 13 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u64>().ok().filter(|n| *n < 1 << 40)
}

fn lane(s: &str) -> Option<Lane> {
    Some(match s {
        "todo" => Lane::Todo,
        "worker" => Lane::Worker,
        "human_decision" => Lane::HumanDecision,
        "human_action" => Lane::HumanAction,
        "done" => Lane::Done,
        _ => return None,
    })
}

fn lane_name(l: Lane) -> &'static str {
    match l {
        Lane::Todo => "todo",
        Lane::Worker => "worker",
        Lane::HumanDecision => "human_decision",
        Lane::HumanAction => "human_action",
        Lane::Done => "done",
    }
}

fn rule_name(r: Rule) -> &'static str {
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

fn run_lane(a: &[String]) -> Option<String> {
    if a.len() != 20 {
        return None;
    }
    let from = lane(&a[1])?;
    let panes = match a[2].as_str() {
        "unknown" => Panes::Unknown,
        "absent" => Panes::Absent,
        "closed" => Panes::Closed,
        "live" => Panes::Live,
        _ => return None,
    };
    let remaining = match a[8].as_str() {
        "empty" => Remaining::Empty,
        "nothing" => Remaining::Nothing,
        "other" => Remaining::Other,
        _ => return None,
    };
    let f = TaskFacts {
        panes,
        idle: yn(&a[3])?,
        working: yn(&a[4])?,
        close_failed: yn(&a[5])?,
        accepted: yn(&a[6])?,
        req_accepted: yn(&a[7])?,
        remaining,
        waiting: yn(&a[9])?,
        answered: yn(&a[10])?,
        detected: yn(&a[11])?,
        detected_at: num(&a[12])?,
        seen: yn(&a[13])?,
        cleared: yn(&a[14])?,
    };
    let s = replace::StopFacts {
        replaced_at: num(&a[15])?,
        woke_at: num(&a[16])?,
        exhausted: yn(&a[17])?,
    };
    let (now, cap) = (num(&a[18])?, num(&a[19])?);
    Some(match replace::decide(from, f, s, now, cap) {
        replace::Verdict::Stay => "none".into(),
        replace::Verdict::Replace => "replace".into(),
        replace::Verdict::Move(m) => format!("move {} {}", rule_name(m.rule), lane_name(m.dest)),
    })
}

fn run_stalled(a: &[String]) -> Option<String> {
    if a.len() != 6 {
        return None;
    }
    let f = StallFacts {
        idle: yn(&a[1])?,
        report: num(&a[2])?,
        assign: num(&a[3])?,
    };
    Some(match stalled(f, num(&a[4])?, num(&a[5])?) {
        Stall::Active => "active".into(),
        Stall::Due(t) => format!("due {t}"),
        Stall::Stalled(Origin::Report) => "stalled report".into(),
        Stall::Stalled(Origin::Assign) => "stalled assign".into(),
    })
}

// bash の resend_step と同じ解釈: agent は claude 以外 Other、rc は 7 だけ true、own / pasted は yes だけ true、delivered は語彙外 Uncertain
fn run_resend(a: &[String]) -> Option<String> {
    if a.len() != 12 {
        return None;
    }
    let input = match a[7].as_str() {
        "empty" => Input::Empty,
        "draft" => Input::Draft,
        "unknown" => Input::Unknown,
        _ => return None,
    };
    let f = ResendFacts {
        tried: yn(&a[1])?,
        last_nudge: yn(&a[2])?,
        nudge_sent: yn(&a[3])?,
        blocked: yn(&a[4])?,
        agent: if a[5] == "claude" {
            Agent::Claude
        } else {
            Agent::Other
        },
        enter_used: a[6] == "7",
        input,
        own: a[8] == "yes",
        pasted: a[9] == "yes",
        delivered: match a[10].as_str() {
            "undelivered" => Delivered::Undelivered,
            "delivered" => Delivered::Executed,
            _ => Delivered::Uncertain,
        },
        overdue: yn(&a[11])?,
    };
    Some(match resend_step(f) {
        ResendStep::Enter => "1a".into(),
        ResendStep::Check(s) => format!(
            "1b:{}",
            match s {
                Skip::Codex => "codex",
                Skip::EnterUsed => "enter_used",
                Skip::Unreadable => "unreadable",
                Skip::Empty => "empty",
                Skip::Pasted => "pasted",
                Skip::Other => "other",
                Skip::Restored => "restored",
                Skip::Uncertain => "uncertain",
            }
        ),
        ResendStep::Nudge => "nudge".into(),
        ResendStep::Block(b) => format!(
            "block:{}",
            match b {
                Block::Codex => "codex",
                Block::Overdue => "overdue",
                Block::Draft => "draft",
            }
        ),
        ResendStep::Exhausted => "exhausted".into(),
        ResendStep::Wait => "none".into(),
    })
}

// lane / stalled / resend の 1 行分。第 1 引数がそれ以外なら None
fn run_extra(a: &[String]) -> Option<String> {
    match a.first().map(String::as_str) {
        Some("lane") => run_lane(a),
        Some("stalled") => run_stalled(a),
        Some("resend") => run_resend(a),
        _ => None,
    }
}

fn run_batch(input: &str) -> Result<Vec<String>, usize> {
    let mut out = Vec::new();
    for (i, line) in input.lines().enumerate() {
        let a: Vec<String> = line.split_whitespace().map(String::from).collect();
        if a.is_empty() {
            continue;
        }
        out.push(run_extra(&a).ok_or(i + 1)?);
    }
    Ok(out)
}

fn usage_exit() -> ! {
    eprintln!("{USAGE}");
    exit(2);
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() == 1 && (a[0] == "-h" || a[0] == "--help") {
        println!("{USAGE}");
        return;
    }
    match a.first().map(String::as_str) {
        Some("lane" | "stalled" | "resend") => match run_extra(&a) {
            Some(s) => println!("{s}"),
            None => usage_exit(),
        },
        Some("batch") if a.len() == 1 => {
            let mut input = String::new();
            if std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).is_err() {
                usage_exit();
            }
            match run_batch(&input) {
                Ok(v) => v.iter().for_each(|s| println!("{s}")),
                Err(n) => {
                    eprintln!("line {n}: bad arguments\n{USAGE}");
                    exit(2);
                }
            }
        }
        _ => run_old(&a),
    }
}

fn run_old(a: &[String]) {
    match parse(a) {
        Some((m, op, o)) => println!("{}", render(m, op, o)),
        None => usage_exit(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &str) -> String {
        let a: Vec<String> = args.split(' ').map(String::from).collect();
        let (m, op, o) = parse(&a).expect("parse");
        render(m, op, o)
    }

    fn head(args: &str) -> String {
        run(args).lines().next().unwrap().to_string()
    }

    #[test]
    fn tag_vocabulary() {
        // 返信不要は reply completion progress info conflict だけ。それ以外 (語彙外・大文字を含む) は返信必須
        for t in ["reply", "completion", "progress", "info", "conflict"] {
            assert_eq!(
                head(&format!("ack {t} new alive valid no")),
                "moved cur none",
                "{t}"
            );
        }
        for t in [
            "escalation",
            "request",
            "alert",
            "none",
            "Escalation",
            "interrupt",
            "",
        ] {
            assert_eq!(
                head(&format!("ack {t} new alive valid no")),
                "refused needs-reply",
                "{t}"
            );
        }
    }

    #[test]
    fn ack_no_reply() {
        // ヘッダの正否に依らず escalation / request は閉じない
        for h in ["valid", "invalid"] {
            assert_eq!(
                head(&format!("ack-no-reply escalation new alive {h} no")),
                "refused needs-reply"
            );
            assert_eq!(
                head(&format!("ack-no-reply request hold alive {h} no")),
                "refused needs-reply"
            );
        }
        assert_eq!(
            head("ack-no-reply alert new alive valid no"),
            "moved cur no-reply"
        );
        assert_eq!(
            head("ack-no-reply none new alive valid no"),
            "moved cur no-reply"
        );
        assert_eq!(run("ack-no-reply alert hold alive valid no"), "moved cur no-reply\neffect append-log no-reply\neffect move-to cur\neffect remove-wait");
    }

    #[test]
    fn ack_sender_gone() {
        assert_eq!(
            head("ack-sender-gone escalation new gone valid no"),
            "moved cur sender-gone"
        );
        assert_eq!(
            head("ack-sender-gone escalation new alive valid no"),
            "refused sender-alive"
        );
        assert_eq!(
            head("ack-sender-gone escalation new unreadable valid no"),
            "refused sender-unknown"
        );
        assert_eq!(
            head("ack-sender-gone escalation new gone invalid no"),
            "refused bad-header"
        );
    }

    #[test]
    fn cur_is_idempotent() {
        // cur の判定はどの検査よりも先 (二重 ack の成功)
        for op in ["ack", "ack-no-reply", "ack-sender-gone"] {
            assert_eq!(
                run(&format!("{op} escalation cur unreadable invalid no")),
                "unchanged",
                "{op}"
            );
        }
        assert_eq!(head("hold escalation cur alive valid no"), "refused in-cur");
        assert_eq!(
            run("reply escalation cur alive valid yes"),
            "unchanged\neffect append-log replied"
        );
    }

    #[test]
    fn reply_and_hold() {
        assert_eq!(
            head("reply escalation new alive invalid yes"),
            "refused bad-header"
        );
        assert_eq!(
            head("reply escalation new alive valid no"),
            "refused send-failed"
        );
        assert_eq!(
            run("reply escalation hold alive valid yes"),
            "moved cur replied\neffect move-to cur\neffect remove-wait\neffect append-log replied"
        );
        assert_eq!(
            run("hold alert new alive valid no"),
            "moved hold none\neffect write-wait\neffect move-to hold\neffect append-log held"
        );
        assert_eq!(
            run("hold alert hold alive valid no"),
            "unchanged\neffect write-wait"
        );
    }

    fn extra(s: &str) -> Option<String> {
        run_extra(&s.split(' ').map(String::from).collect::<Vec<_>>())
    }

    #[test]
    fn lane_cmd() {
        let base = "lane todo live no no no no no nothing no no no 0 no no 0 0 no 100000 30";
        assert_eq!(extra(base).unwrap(), "move live worker");
        assert_eq!(
            extra("lane done live no no no no no nothing no no no 0 no no 0 0 no 100000 30")
                .unwrap(),
            "none"
        );
        assert_eq!(
            extra("lane worker absent no no no yes no other yes no no 0 no no 0 0 no 100000 30")
                .unwrap(),
            "move settle-gone human_decision"
        );
        assert_eq!(
            extra("lane worker live yes no no no no other no no yes 98200 yes no 0 0 no 100000 30")
                .unwrap(),
            "replace"
        );
        // 新しい 3 引数 (replaced_at / woke_at / exhausted) の位置
        assert_eq!(
            extra("lane worker live yes no no no no other no no yes 98200 yes no 90000 0 no 100000 30")
                .unwrap(),
            "move stall-cap human_action"
        );
        assert_eq!(
            extra("lane worker live yes no no no no other no no yes 99000 yes no 0 0 yes 100000 30")
                .unwrap(),
            "replace"
        );
        assert_eq!(
            extra("lane worker live yes no no no no other no no yes 98200 yes no 0 99000 no 100000 30")
                .unwrap(),
            "none"
        );
        // 誤り: bool 語彙外、lane 語彙外、数値の桁あふれ、引数の数
        assert!(extra(&base.replace("live no", "live maybe")).is_none());
        assert!(extra(&base.replacen("todo", "nowhere", 1)).is_none());
        assert!(extra(&base.replace("100000", "1099511627776")).is_none());
        assert!(extra(&base.replace("100000", "-1")).is_none());
        assert!(extra("lane todo").is_none());
    }

    #[test]
    fn stalled_cmd() {
        assert_eq!(extra("stalled yes 1000 500 1000 10").unwrap(), "due 1600");
        assert_eq!(
            extra("stalled yes 1000 500 1600 10").unwrap(),
            "stalled report"
        );
        assert_eq!(
            extra("stalled yes 500 1000 2000 10").unwrap(),
            "stalled assign"
        );
        assert_eq!(extra("stalled no 1000 500 9999 10").unwrap(), "active");
        assert!(extra("stalled yes x 500 1000 10").is_none());
        assert!(extra("stalled yes 1 2 3").is_none());
    }

    #[test]
    fn resend_cmd() {
        assert_eq!(
            extra("resend no no no no claude 0 draft yes no undelivered no").unwrap(),
            "1a"
        );
        assert_eq!(
            extra("resend no no no no claude 7 draft yes no undelivered no").unwrap(),
            "1b:enter_used"
        );
        assert_eq!(
            extra("resend no no no no claude 0 draft yes no whatever no").unwrap(),
            "1b:uncertain"
        );
        assert_eq!(
            extra("resend yes no no no claude 0 empty no no x no").unwrap(),
            "nudge"
        );
        assert_eq!(
            extra("resend yes no no no gemini 0 empty no no x no").unwrap(),
            "block:codex"
        );
        assert_eq!(
            extra("resend yes yes no no claude 0 empty no no x no").unwrap(),
            "exhausted"
        );
        assert_eq!(
            extra("resend yes no no no claude 0 unknown no no x no").unwrap(),
            "none"
        );
        assert!(extra("resend yes no no no claude 0 bogus no no x no").is_none());
        assert!(extra("resend maybe no no no claude 0 empty no no x no").is_none());
    }

    #[test]
    fn batch_cmd() {
        let ok = run_batch("stalled no 1 2 3 4\n\n  stalled yes 1000 0 1000 10\n").unwrap();
        assert_eq!(ok, ["active", "due 1600"]);
        assert_eq!(run_batch("stalled no 1 2 3 4\n\nbogus\n").unwrap_err(), 3);
    }

    #[test]
    fn bad_args() {
        let a: Vec<String> = "ack escalation nowhere alive valid no"
            .split(' ')
            .map(String::from)
            .collect();
        assert!(parse(&a).is_none());
        assert!(parse(&[]).is_none());
    }
}
