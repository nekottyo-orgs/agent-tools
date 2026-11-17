// 判定部。lean/HerdrTimeline.lean のモデルと名前・順序・判定を揃える。
// 一致はモデルの証明でなく、下の #[cfg(test)] が同じ入力の例で確かめる。
// 時刻はモデルと同じく秒 (ミリ秒は捨てる)。同じ秒の前後は (src, seq) で決まる。

use crate::text::{clean, first_line};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Instr,
    Msg,
    Pr,
    Report,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Instr => "指示",
            Kind::Msg => "from",
            Kind::Pr => "PR",
            Kind::Report => "報告",
        }
    }
}

/// src は源 (0 = transcript、1 = inbox)、seq は源の中の連番 (transcript は行番号、inbox はファイル名順の位置)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub ts: i64,
    pub src: u8,
    pub seq: usize,
    pub kind: Kind,
    pub body: String,
}

pub fn newer_eq(a: &Event, b: &Event) -> bool {
    a.ts > b.ts || (a.ts == b.ts && (a.src > b.src || (a.src == b.src && a.seq >= b.seq)))
}

pub fn order(mut es: Vec<Event>) -> Vec<Event> {
    es.sort_by(|a, b| match (newer_eq(a, b), newer_eq(b, a)) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        _ => Ordering::Greater,
    });
    es
}

/// 隣り合う同じ種類をまとめる。代表は最初 (最も新しい) のイベント
pub fn group(es: &[Event]) -> Vec<(Event, usize)> {
    let mut out: Vec<(Event, usize)> = Vec::new();
    for e in es {
        match out.last_mut() {
            Some((g, n)) if g.kind == e.kind => *n += 1,
            _ => out.push((e.clone(), 1)),
        }
    }
    out
}

pub fn total(gs: &[(Event, usize)]) -> usize {
    gs.iter().map(|g| g.1).sum()
}

/// JSON を解釈した後の transcript の 1 行。user / pr-link 以外の行も行番号を保つために空の Line で置く
#[derive(Clone, Debug, Default)]
pub struct Line {
    pub ts: Option<i64>,
    pub typ: String,
    pub is_meta: bool,
    pub is_sidechain: bool,
    pub is_compact_summary: bool,
    pub content: Option<String>,
    pub pr: Option<String>,
    pub pr_label: String,
}

pub fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\n' | '\t' | '\r')
}

pub fn trim_left(s: &str) -> &str {
    s.trim_start_matches(is_ws)
}

const SKIP_PREFIXES: [&str; 3] = [
    "<",
    "Stop hook feedback:",
    "This session is being continued",
];
const PASTE_OPEN: &str = "<pasted_content";
const HERDR_TAG: &str = "[herdr-message] from ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    HerdrMsg,
    Paste,
    Instr,
    Skip,
}

pub fn classify(s: &str) -> Class {
    let t = trim_left(s);
    if t.contains(HERDR_TAG) {
        Class::HerdrMsg
    } else if t.starts_with(PASTE_OPEN) {
        Class::Paste
    } else if t.is_empty() || SKIP_PREFIXES.iter().any(|p| t.starts_with(p)) {
        Class::Skip
    } else {
        Class::Instr
    }
}

pub fn excluded(l: &Line) -> bool {
    l.is_meta || l.is_sidechain || l.is_compact_summary
}

pub fn user_event(i: usize, l: &Line) -> Option<Event> {
    let (ts, c) = (l.ts?, l.content.as_deref()?);
    if excluded(l) {
        return None;
    }
    let (kind, body) = match classify(c) {
        Class::HerdrMsg => (Kind::Msg, herdr_body(c)),
        Class::Paste => (Kind::Instr, paste_body(c)),
        Class::Instr => (Kind::Instr, first_line(c)),
        Class::Skip => return None,
    };
    let body = clean(&body);
    // モデルは本文の中身を扱わないので、中身の無い貼り付けを落とすのはここだけの判定
    if body.is_empty() {
        return None;
    }
    Some(Event {
        ts,
        src: 0,
        seq: i,
        kind,
        body,
    })
}

/// 「名前: ヘッダ行の次の空でない行」。名前は試作の `from (.+?) \(([^)]*)\)` と同じく最初の " (" まで
fn herdr_body(c: &str) -> String {
    let Some(at) = c.find(HERDR_TAG) else {
        return first_line(c);
    };
    let start = at + HERDR_TAG.len();
    let rest = &c[start..];
    for (idx, _) in rest.match_indices(" (") {
        let name = &rest[..idx];
        if name.contains('\n') {
            break;
        }
        if idx == 0 {
            continue;
        }
        let Some(close) = rest[idx + 2..].find(')') else {
            break;
        };
        let after = &rest[idx + 2 + close + 1..];
        let next = after
            .split_once('\n')
            .map(|(_, r)| first_line(r))
            .unwrap_or_default();
        return format!("{name}: {next}");
    }
    first_line(c)
}

/// wrapper のタグ (<pasted_content ...> と </pasted_content>) を外した中身の最初の空でない行
fn paste_body(c: &str) -> String {
    let mut s = c.to_string();
    for open in ["</pasted_content", "<pasted_content"] {
        while let Some(i) = s.find(open) {
            let end = s[i..].find('>').map_or(s.len(), |e| i + e + 1);
            s.replace_range(i..end, "\n");
        }
    }
    first_line(&s)
}

/// PR は URL の初出だけ
pub fn line_events(lines: &[Line]) -> Vec<Event> {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if l.typ == "pr-link" {
            if let (Some(ts), Some(u)) = (l.ts, l.pr.as_deref()) {
                if !seen.contains(&u) {
                    seen.push(u);
                    out.push(Event {
                        ts,
                        src: 0,
                        seq: i,
                        kind: Kind::Pr,
                        body: clean(&l.pr_label),
                    });
                }
            }
        } else if l.typ == "user" {
            out.extend(user_event(i, l));
        }
    }
    out
}

/// transcript を読めなければ None。そのときイベントは 0 件
pub fn transcript_events(input: Option<&[Line]>) -> Vec<Event> {
    match input {
        None => Vec::new(),
        Some(ls) => line_events(ls),
    }
}

pub struct Msg {
    pub from_pane: String,
    pub from_socket: Option<String>,
    pub ts: i64,
    pub tag: String,
    pub body: String,
}

/// since は transcript の最初の時刻。pane id が別 session で再利用されたときに古い報告を拾わないため
pub struct Pane {
    pub pane: String,
    pub socket: String,
    pub since: Option<i64>,
}

/// from_socket の無い旧形式は、どの session の pane か決められないので帰属させない
pub fn attributed(p: &Pane, m: &Msg) -> bool {
    m.from_socket.as_deref() == Some(p.socket.as_str())
        && m.from_pane == p.pane
        && p.since.is_none_or(|s| s <= m.ts)
}

fn allowed(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// session_id はファイル名に join するので、区切りや . / .. になり得るものを弾く
pub fn valid_sid(s: &str) -> bool {
    match s.chars().next() {
        None => false,
        Some(c) => c != '.' && s.chars().all(allowed),
    }
}

/// 出し直しに追随するため、最初でなく最後に指示された brief を正とする
pub fn last_brief(xs: &[Option<String>]) -> Option<String> {
    xs.iter().flatten().next_back().cloned()
}

/// names はその時点で済んだ step 名の一覧 (差分でなく丸ごと)
pub struct Progress {
    pub ts: i64,
    pub seq: usize,
    pub names: Vec<String>,
}

pub fn prog_newer_eq(a: &Progress, b: &Progress) -> bool {
    a.ts > b.ts || (a.ts == b.ts && a.seq >= b.seq)
}

pub fn latest(ps: &[Progress]) -> Option<&Progress> {
    ps.iter().rev().fold(None, |acc, p| match acc {
        None => Some(p),
        Some(q) => Some(if prog_newer_eq(p, q) { p } else { q }),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Unreported,
    Reported { done: Vec<String>, unknown: usize },
}

/// brief に無い step 名は済みに数えず、件数だけ残す
pub fn status(steps: &[String], ps: &[Progress]) -> Status {
    match latest(ps) {
        None => Status::Unreported,
        Some(p) => Status::Reported {
            done: steps
                .iter()
                .filter(|s| p.names.contains(s))
                .cloned()
                .collect(),
            unknown: p.names.iter().filter(|n| !steps.contains(n)).count(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ts: i64, src: u8, seq: usize, kind: Kind) -> Event {
        Event {
            ts,
            src,
            seq,
            kind,
            body: format!("{ts}-{src}-{seq}"),
        }
    }

    fn user(content: &str) -> Line {
        Line {
            ts: Some(100),
            typ: "user".into(),
            content: Some(content.into()),
            ..Default::default()
        }
    }

    #[test]
    fn order_is_by_ts_src_seq_and_independent_of_input_order() {
        let a = vec![
            ev(5, 0, 3, Kind::Instr),
            ev(5, 1, 0, Kind::Report),
            ev(5, 0, 7, Kind::Msg),
            ev(9, 0, 1, Kind::Pr),
        ];
        let mut b = a.clone();
        b.reverse();
        let keys: Vec<_> = order(a.clone())
            .iter()
            .map(|e| (e.ts, e.src, e.seq))
            .collect();
        assert_eq!(
            keys,
            vec![(9, 0, 1), (5, 1, 0), (5, 0, 7), (5, 0, 3)],
            "同時刻は inbox が先、同じ源は連番の大きい方が先のはず"
        );
        assert_eq!(
            order(a),
            order(b),
            "入力の順を入れ替えても同じ結果になるはず"
        );
    }

    #[test]
    fn group_total_equals_event_count() {
        let es = vec![
            ev(9, 0, 0, Kind::Instr),
            ev(8, 0, 1, Kind::Instr),
            ev(7, 1, 0, Kind::Report),
            ev(6, 0, 2, Kind::Instr),
        ];
        let gs = group(&es);
        assert_eq!(
            gs.iter().map(|g| g.1).collect::<Vec<_>>(),
            vec![2, 1, 1],
            "隣り合う同じ種類だけがまとまるはず"
        );
        assert_eq!(total(&gs), es.len(), "まとめても合計は元の件数と等しいはず");
        assert!(
            gs.windows(2).all(|w| w[0].0.kind != w[1].0.kind),
            "まとめた後で隣り合う行は種類が違うはず"
        );
    }

    #[test]
    fn req2_each_exclusion_hides_and_removing_it_shows() {
        type Set = fn(&mut Line);
        let cases: [(&str, Set); 3] = [
            ("isMeta", |l| l.is_meta = true),
            ("isSidechain", |l| l.is_sidechain = true),
            ("isCompactSummary", |l| l.is_compact_summary = true),
        ];
        for (name, set) in cases {
            let mut l = user("調べて");
            set(&mut l);
            assert!(
                user_event(0, &l).is_none(),
                "{name} だけが付いた行は出ないはず"
            );
            assert!(
                user_event(0, &user("調べて")).is_some(),
                "{name} を外した同じ行は出るはず"
            );
        }
    }

    #[test]
    fn req4_leading_whitespace_does_not_change_classification() {
        for s in [
            "<pasted_content>\n中身の行\n</pasted_content>",
            "Stop hook feedback: x",
            "<command-name>",
            "This session is being continued",
            "直して",
            "[herdr-message] from 窓口 (wA:p1)\n本文",
        ] {
            for ws in ["\n", "  ", "\t\r\n "] {
                let padded = format!("{ws}{s}");
                assert_eq!(
                    classify(&padded),
                    classify(s),
                    "先頭に空白を足しても判定は同じはず: {s:?}"
                );
                assert_eq!(
                    user_event(0, &user(&padded)).map(|e| e.body),
                    user_event(0, &user(s)).map(|e| e.body),
                    "本文も同じはず: {s:?}"
                );
            }
        }
        assert_eq!(
            classify("Stop hook feedback: x"),
            Class::Skip,
            "skip 前置きは出さないはず"
        );
        let e = user_event(
            0,
            &user("\n\n<pasted_content type=\"text\">\n\n中身の行\n二行目\n</pasted_content>"),
        )
        .expect("貼り付けは出るはず");
        assert_eq!(
            (e.kind, e.body.as_str()),
            (Kind::Instr, "中身の行"),
            "貼り付けは中身の最初の行が指示として出るはず"
        );
        assert!(
            user_event(0, &user("<pasted_content>\n</pasted_content>")).is_none(),
            "中身の無い貼り付けは出ないはず"
        );
    }

    #[test]
    fn herdr_message_body_is_name_and_next_line() {
        let e = user_event(
            0,
            &user("[herdr-message] from 調査役 (wA:p2)\n\n終わった。表を置いた\n"),
        )
        .expect("from は出るはず");
        assert_eq!(
            (e.kind, e.body.as_str()),
            (Kind::Msg, "調査役: 終わった。表を置いた"),
            "名前と次の行になるはず"
        );
    }

    #[test]
    fn pr_link_only_first_url() {
        let pr = |url: &str| Line {
            ts: Some(1),
            typ: "pr-link".into(),
            pr: Some(url.into()),
            pr_label: "o/r#1".into(),
            ..Default::default()
        };
        let es = line_events(&[pr("u1"), Line::default(), pr("u1"), pr("u2")]);
        assert_eq!(
            es.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![0, 3],
            "同じ URL の 2 回目は出ず、連番は行番号のはず"
        );
    }

    #[test]
    fn req1_unreadable_transcript_has_no_events() {
        assert!(
            transcript_events(None).is_empty(),
            "読めない transcript はイベント 0 件のはず"
        );
    }

    #[test]
    fn req3_attribution_needs_socket_pane_and_session_start() {
        let p = Pane {
            pane: "wA:p1".into(),
            socket: "/s/a".into(),
            since: Some(100),
        };
        let m = |sock: Option<&str>, ts: i64| Msg {
            from_pane: "wA:p1".into(),
            from_socket: sock.map(Into::into),
            ts,
            tag: String::new(),
            body: String::new(),
        };
        assert!(
            attributed(&p, &m(Some("/s/a"), 100)),
            "socket と pane が一致し開始以降なら帰属するはず"
        );
        assert!(
            !attributed(&p, &m(Some("/s/b"), 200)),
            "別 socket の同じ pane id は帰属しないはず"
        );
        assert!(
            !attributed(&p, &m(None, 200)),
            "from_socket の無い旧形式は帰属しないはず"
        );
        assert!(
            !attributed(&p, &m(Some("/s/a"), 99)),
            "session 開始前の msg は帰属しないはず"
        );
    }

    #[test]
    fn req8_valid_sid_rejects_path_like_values() {
        for bad in ["../x", "a/b", ".", "..", ".hidden", "", "a b"] {
            assert!(!valid_sid(bad), "{bad:?} は弾くはず");
        }
        assert!(
            valid_sid("0b5c1f3e-7a2d-4c9e-8f10-123456789abc"),
            "UUID 形は通すはず"
        );
    }

    #[test]
    fn req9_last_brief_wins() {
        let xs = vec![Some("a".to_string()), None, Some("b".to_string()), None];
        assert_eq!(
            last_brief(&xs).as_deref(),
            Some("b"),
            "最後に指示された brief を使うはず"
        );
        assert_eq!(
            last_brief(&[None, None]),
            None,
            "brief が無ければ無しのはず"
        );
    }

    fn prog(ts: i64, seq: usize, names: &[&str]) -> Progress {
        Progress {
            ts,
            seq,
            names: names.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn progress_status() {
        let steps: Vec<String> = ["調査", "対応表", "設計案"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            status(&steps, &[]),
            Status::Unreported,
            "1 通も無ければ未報告のはず"
        );
        assert_eq!(
            status(&steps, &[prog(1, 0, &[])]),
            Status::Reported {
                done: vec![],
                unknown: 0
            },
            "空の報告は 0/M で未報告と区別されるはず"
        );
        let a = [
            prog(1, 0, &["調査"]),
            prog(2, 1, &["対応表", "調査", "別件"]),
        ];
        let b = [
            prog(2, 1, &["対応表", "調査", "別件"]),
            prog(1, 0, &["調査"]),
        ];
        let want = Status::Reported {
            done: vec!["調査".into(), "対応表".into()],
            unknown: 1,
        };
        assert_eq!(
            status(&steps, &a),
            want,
            "最新の報告を brief の順で数え、brief に無い名前は件数だけのはず"
        );
        assert_eq!(status(&steps, &b), want, "届く順に依らないはず");
        let same_ts = [prog(5, 1, &["調査"]), prog(5, 0, &["調査", "対応表"])];
        assert_eq!(
            status(&steps, &same_ts),
            Status::Reported {
                done: vec!["調査".into()],
                unknown: 0
            },
            "同時刻はファイル名順の位置が大きい方が最新のはず"
        );
    }
}
