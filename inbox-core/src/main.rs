// inbox-core の CLI。run-inbox.sh が msg 1 通ごとに観測を引数で渡し、判定を stdout の 1 行目で受け取る。
// 2 行目以降は判定から決まる副作用の列 (plan)。拒否も判定の 1 つなので exit 0 で返し、
// exit 2 は引数の誤りだけに使う。run-inbox.sh は exit 0 以外と形の崩れた出力を「異常」とみなして bash の判定へ戻る。
use inbox_core::*;
use std::process::exit;

const USAGE: &str = "使い方: inbox-core <op> <tag> <box> <sender> <header> <sent>
  op:     ack | ack-no-reply | ack-sender-gone | reply | hold
  tag:    msg 名から切り出したタグ (語彙外は返信必須として扱う)
  box:    new | hold | cur
  sender: alive | gone | unreadable (ack-sender-gone 以外は読まない)
  header: valid | invalid (ack-sender-gone と reply だけが読む)
  sent:   yes | no (reply だけが読む)
出力: 1 行目 moved <box> <res> | unchanged | refused <理由>、2 行目以降 effect <副作用>";

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
    let m = Msg { tag: tag(&a[1]), sender: 0, place, res: Res::None };
    Some((m, op, Obs { sender, sent, header_valid }))
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

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() == 1 && (a[0] == "-h" || a[0] == "--help") {
        println!("{USAGE}");
        return;
    }
    match parse(&a) {
        Some((m, op, o)) => println!("{}", render(m, op, o)),
        None => {
            eprintln!("{USAGE}");
            exit(2);
        }
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
            assert_eq!(head(&format!("ack {t} new alive valid no")), "moved cur none", "{t}");
        }
        for t in ["escalation", "request", "alert", "none", "Escalation", "interrupt", ""] {
            assert_eq!(head(&format!("ack {t} new alive valid no")), "refused needs-reply", "{t}");
        }
    }

    #[test]
    fn ack_no_reply() {
        // ヘッダの正否に依らず escalation / request は閉じない
        for h in ["valid", "invalid"] {
            assert_eq!(head(&format!("ack-no-reply escalation new alive {h} no")), "refused needs-reply");
            assert_eq!(head(&format!("ack-no-reply request hold alive {h} no")), "refused needs-reply");
        }
        assert_eq!(head("ack-no-reply alert new alive valid no"), "moved cur no-reply");
        assert_eq!(head("ack-no-reply none new alive valid no"), "moved cur no-reply");
        assert_eq!(run("ack-no-reply alert hold alive valid no"), "moved cur no-reply\neffect append-log no-reply\neffect move-to cur\neffect remove-wait");
    }

    #[test]
    fn ack_sender_gone() {
        assert_eq!(head("ack-sender-gone escalation new gone valid no"), "moved cur sender-gone");
        assert_eq!(head("ack-sender-gone escalation new alive valid no"), "refused sender-alive");
        assert_eq!(head("ack-sender-gone escalation new unreadable valid no"), "refused sender-unknown");
        assert_eq!(head("ack-sender-gone escalation new gone invalid no"), "refused bad-header");
    }

    #[test]
    fn cur_is_idempotent() {
        // cur の判定はどの検査よりも先 (二重 ack の成功)
        for op in ["ack", "ack-no-reply", "ack-sender-gone"] {
            assert_eq!(run(&format!("{op} escalation cur unreadable invalid no")), "unchanged", "{op}");
        }
        assert_eq!(head("hold escalation cur alive valid no"), "refused in-cur");
        assert_eq!(run("reply escalation cur alive valid yes"), "unchanged\neffect append-log replied");
    }

    #[test]
    fn reply_and_hold() {
        assert_eq!(head("reply escalation new alive invalid yes"), "refused bad-header");
        assert_eq!(head("reply escalation new alive valid no"), "refused send-failed");
        assert_eq!(run("reply escalation hold alive valid yes"), "moved cur replied\neffect move-to cur\neffect remove-wait\neffect append-log replied");
        assert_eq!(run("hold alert new alive valid no"), "moved hold none\neffect write-wait\neffect move-to hold\neffect append-log held");
        assert_eq!(run("hold alert hold alive valid no"), "unchanged\neffect write-wait");
    }

    #[test]
    fn bad_args() {
        let a: Vec<String> = "ack escalation nowhere alive valid no".split(' ').map(String::from).collect();
        assert!(parse(&a).is_none());
        assert!(parse(&[]).is_none());
    }
}
