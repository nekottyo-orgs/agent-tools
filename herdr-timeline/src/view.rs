// 画面の組み立て。描画する文字列を返すだけで、端末には触らない (試作の layout と同じ見た目を目標にする)。

use crate::core::{group, total, Event, Kind, Status};
use crate::data::{Detail, Goal, Run};
use crate::text::{age_text, clip, fit, fmt_stamp, pad, tl_body, width_of, wrap};
use std::collections::HashMap;

const NAME_MAX: usize = 44;
const WORK_MIN: usize = 8; // 要約に最低限必要な幅。足りない行は要約を省く
const PANE_ID_MAX: usize = 20; // 行末の pane id の列幅の上限。別 session の行は session 名が付く
const AGE_W: usize = 4;
const ALERT_STYLE: &str = "1;31";
pub const TL_MAX: usize = 8; // Timeline に出す行数の上限 (まとめた後)。残りは「他 N 件」

pub struct Row {
    pub key: String,
    pub tag: String,
    pub status: String,
    pub name: String,
    pub work: String,
    pub since: Option<i64>,
    /// 経過が観測起点 (viewer が状態を見始めた時刻) なら真。≥ を付ける
    pub approx: bool,
    pub tab_id: Option<String>,
    pub ws: (String, String),
}

/// (key, tab_id) の並び。表示順で、選択の対象になる行だけ
pub type Order = Vec<(String, String)>;

pub struct View<'a> {
    pub rows: &'a [Row],
    pub runs: &'a [Run],
    pub warn: &'a [String],
    pub names: &'a HashMap<String, String>,
    pub ws_names: &'a HashMap<(String, String), String>,
    pub width: usize,
    pub color: bool,
    /// None は選択なし (マーカー列も出さない)。Some は (選択中の key, 前回の表示順)
    pub select: Option<(Option<String>, &'a Order)>,
    pub height: Option<usize>,
    pub now: i64,
    pub expanded: Option<(&'a str, &'a Detail)>,
    pub off: &'a dyn Fn(i64) -> i64,
}

pub fn paint(color: bool, s: &str, code: &str) -> String {
    if color && !code.is_empty() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn symbol(status: &str) -> &'static str {
    match status {
        "blocked" => "▲",
        "working" => "●",
        "idle" => "○",
        "done" => "✓",
        _ => "?",
    }
}

fn kind_color(k: Kind) -> &'static str {
    match k {
        Kind::Instr => "36",
        Kind::Report => "33",
        Kind::Pr => "35",
        Kind::Msg => "34",
    }
}

/// Timeline の表示行。events は新しい順。隣り合う同じ種類を 1 行にまとめ (時刻と本文は最新)、最大 TL_MAX 行
pub fn timeline_lines(
    events: &[Event],
    width: usize,
    color: bool,
    indent: usize,
    now: i64,
    off: &dyn Fn(i64) -> i64,
) -> Vec<String> {
    let groups = group(events);
    let shown = &groups[..groups.len().min(TL_MAX)];
    let sp = " ".repeat(indent);
    let mut out: Vec<String> = shown
        .iter()
        .map(|(e, n)| {
            let mut text = tl_body(e.kind, &e.body);
            if *n > 1 {
                text = format!("×{n} {text}");
            }
            let stamp = fmt_stamp(e.ts, now, off);
            let text = clip(
                &text,
                width.saturating_sub(indent + width_of(&stamp) + 8).max(1),
            );
            let kind = pad(e.kind.label(), 4);
            format!(
                "{sp}{}  {}  {text}",
                paint(color, &stamp, "2"),
                paint(color, &kind, kind_color(e.kind))
            )
        })
        .collect();
    let n = total(shown);
    if n < events.len() {
        out.push(format!(
            "{sp}{}",
            paint(color, &format!("他 {} 件", events.len() - n), "2")
        ));
    }
    out
}

/// brief に steps があるときだけの step の行
pub fn step_line(d: &Goal, width: usize) -> Option<String> {
    if d.steps.is_empty() {
        return None;
    }
    let (head, done, unknown) = match &d.status {
        Some(Status::Reported { done, unknown }) => (
            format!("{}/{}", done.len(), d.steps.len()),
            done.as_slice(),
            *unknown,
        ),
        _ => ("進捗未報告".to_string(), &[][..], 0),
    };
    let marks: Vec<String> = d
        .steps
        .iter()
        .map(|s| format!("{}{s}", if done.contains(s) { "✓" } else { "・" }))
        .collect();
    let mut line = format!("step {head}  {}", marks.join(" "));
    if unknown > 0 {
        line += &format!("  (brief に無い step {unknown} 件)");
    }
    Some(clip(&line, width))
}

fn tally(rows: &[&Row]) -> String {
    let c = |s: &str| rows.iter().filter(|x| x.status == s).count();
    [("done", "done"), ("idle", "idle"), ("unknown", "不明")]
        .iter()
        .filter(|(k, _)| c(k) > 0)
        .map(|(k, l)| format!("{l} {}", c(k)))
        .collect::<Vec<_>>()
        .join("  ")
}

/// 選択する key。sel が order に無ければ、前回の表示順で近い順 (右隣、左隣、右 2 …) に残っている行、無ければ先頭
fn choose(
    order: &[(String, String)],
    sel: Option<&str>,
    prev: &[(String, String)],
) -> Option<String> {
    let ids: Vec<&str> = order.iter().map(|x| x.0.as_str()).collect();
    if ids.is_empty() {
        return None;
    }
    if let Some(s) = sel.filter(|s| ids.contains(s)) {
        return Some(s.to_string());
    }
    let old: Vec<&str> = prev.iter().map(|x| x.0.as_str()).collect();
    if let Some(i) = sel.and_then(|s| old.iter().position(|x| *x == s)) {
        for d in 1..old.len() {
            for j in [i + d, i.wrapping_sub(d)] {
                if j < old.len() && ids.contains(&old[j]) {
                    return Some(old[j].to_string());
                }
            }
        }
    }
    Some(ids[0].to_string())
}

enum Alert<'a> {
    Pane(&'a Row),
    Run(&'a Run),
}

/// (行, 表示順の [(key, tab_id)], 選択された key)
pub fn layout(v: &View) -> (Vec<String>, Vec<(String, String)>, Option<String>) {
    let paint = |s: &str, code: &str| paint(v.color, s, code);
    let blocked: Vec<&Row> = v.rows.iter().filter(|x| x.status == "blocked").collect();
    let working: Vec<&Row> = v.rows.iter().filter(|x| x.status == "working").collect();
    let mut alerts: Vec<(Option<i64>, Alert)> =
        blocked.iter().map(|x| (x.since, Alert::Pane(x))).collect();
    alerts.extend(
        v.runs
            .iter()
            .filter(|r| r.new > 0)
            .map(|r| (r.oldest, Alert::Run(r))),
    );
    // 要対応は長く続いているものから。時間の取れないものは最後
    alerts.sort_by_key(|a| (a.0.is_none(), a.0));
    let ordered: Vec<&Row> = alerts
        .iter()
        .filter_map(|a| {
            if let Alert::Pane(x) = a.1 {
                Some(x)
            } else {
                None
            }
        })
        .chain(working.iter().copied())
        .collect();
    let order: Vec<(String, String)> = ordered
        .iter()
        .filter_map(|x| Some((x.key.clone(), x.tab_id.clone()?)))
        .collect();
    let chosen = v
        .select
        .as_ref()
        .and_then(|(sel, prev)| choose(&order, sel.as_deref(), prev));

    // 列幅は要対応と作業中で揃える。どちらかが全幅を食わないよう名前は 45% までにする
    let shown: Vec<&Row> = blocked.iter().chain(working.iter()).copied().collect();
    let inner = v.width as isize - 2;
    let pw = shown
        .iter()
        .map(|x| width_of(&x.tag))
        .max()
        .unwrap_or(0)
        .min(PANE_ID_MAX) as isize;
    let key_w: isize = if v.select.is_some() { 2 } else { 0 };
    let avail = inner - 4 - key_w - 2 - AGE_W as isize - 2 - (1 + pw);
    let nw = shown
        .iter()
        .map(|x| width_of(&x.name))
        .max()
        .unwrap_or(0)
        .min(NAME_MAX)
        .min(8.max(avail * 45 / 100) as usize);

    let who = |pane: &str| {
        v.names.get(pane).cloned().unwrap_or_else(|| {
            if pane.is_empty() {
                "-".into()
            } else {
                pane.to_string()
            }
        })
    };
    let age = |x: &Row| {
        let t = age_text(x.since.map(|s| v.now - s));
        pad(
            &format!("{}{t}", if x.approx && !t.is_empty() { "≥" } else { "" }),
            AGE_W,
        )
    };
    let mut keep = None;
    let mut blocks: Vec<Vec<String>> = Vec::new();

    // 名前と要約を列の中で折り返し、2 行目以降は同じ桁から始める。whole (要対応) は行全体を赤太字にする
    let mut pane_block = |x: &Row, indent: usize, whole: bool, blocks: &mut Vec<Vec<String>>| {
        let key = match &v.select {
            None => String::new(),
            Some(_) if chosen.as_deref() == Some(x.key.as_str()) => "> ".into(),
            Some(_) => "  ".into(),
        };
        let sw =
            inner - indent as isize - key_w - (2 + nw as isize + 1 + AGE_W as isize) - 1 - (1 + pw);
        let names = wrap(&x.name, nw, nw);
        let works = if sw >= WORK_MIN as isize && !x.work.is_empty() {
            wrap(&x.work, sw as usize, sw as usize)
        } else {
            Vec::new()
        };
        let blank = " ".repeat(indent);
        let mut lines = Vec::new();
        for i in 0..names.len().max(works.len()).max(1) {
            let first = i == 0;
            let nm = pad(names.get(i).map_or("", String::as_str), nw);
            let mid = if sw >= WORK_MIN as isize {
                format!(
                    " {}",
                    pad(works.get(i).map_or("", String::as_str), sw as usize)
                )
            } else {
                String::new()
            };
            let lead = if first {
                format!("{key}{} ", symbol(&x.status))
            } else {
                " ".repeat(key_w as usize + 2)
            };
            let ag = if first { age(x) } else { " ".repeat(AGE_W) };
            if whole {
                let mut body = format!("{lead}{nm} {ag}{mid}");
                if first {
                    body += &format!(" {}", x.tag);
                }
                lines.push(blank.clone() + &paint(body.trim_end(), ALERT_STYLE));
            } else {
                let tag = if first {
                    paint(&format!(" {}", x.tag), "2")
                } else {
                    String::new()
                };
                lines.push(format!("{blank}{lead}{} {ag}{mid}{tag}", paint(&nm, "1")));
            }
        }
        if let Some((_, d)) = v.expanded.filter(|(k, _)| *k == x.key) {
            lines.extend(
                step_line(&d.goal, v.width.saturating_sub(6))
                    .map(|s| format!("      {}", paint(&s, "2"))),
            );
            lines.extend(timeline_lines(&d.events, v.width, v.color, 6, v.now, v.off));
        }
        if chosen.as_deref() == Some(x.key.as_str()) {
            keep = Some(blocks.len());
        }
        blocks.push(lines);
    };
    let head = |t: &str, code: &str| paint(&clip(t, v.width), code);
    let para = |t: &str, indent: usize, code: &str, hang: usize| -> Vec<String> {
        wrap(
            t,
            v.width.saturating_sub(indent),
            v.width.saturating_sub(indent + hang),
        )
        .iter()
        .enumerate()
        .map(|(i, l)| {
            format!(
                "{}{}",
                " ".repeat(if i == 0 { indent } else { indent + hang }),
                paint(l, code)
            )
        })
        .collect()
    };

    if !v.warn.is_empty() {
        blocks.push(para(
            &format!("!! 取得不全: {}", v.warn.join(" / ")),
            0,
            ALERT_STYLE,
            3,
        ));
    }
    if alerts.is_empty() {
        blocks.push(vec![head("■ 要対応なし", "1")]);
    } else {
        blocks.push(vec![head(
            &format!("■ 要対応 {}", alerts.len()),
            ALERT_STYLE,
        )]);
        for (since, a) in &alerts {
            match a {
                Alert::Pane(x) => pane_block(x, 2, true, &mut blocks),
                Alert::Run(r) => {
                    let t = age_text(since.map(|s| v.now - s));
                    let t = if t.is_empty() { t } else { format!("  {t}") };
                    blocks.push(para(
                        &format!("▲ 窓口 {}  未処理の報告 {} 件{t}", who(&r.orch), r.new),
                        2,
                        ALERT_STYLE,
                        2,
                    ));
                }
            }
        }
    }
    // 作業中は pane id 順で固定する。経過や要約では並べ替えず、状態が変わるまで行が動かない
    blocks.push(vec![head(
        &if working.is_empty() {
            "■ 作業中なし".into()
        } else {
            format!("■ 作業中 {}", working.len())
        },
        "1",
    )]);
    for x in &working {
        pane_block(x, 2, false, &mut blocks);
    }
    let mut ws: Vec<((String, String), Vec<&Row>)> = Vec::new();
    for x in v
        .rows
        .iter()
        .filter(|x| x.status != "blocked" && x.status != "working")
    {
        match ws.iter_mut().find(|(k, _)| *k == x.ws) {
            Some((_, rs)) => rs.push(x),
            None => ws.push((x.ws.clone(), vec![x])),
        }
    }
    ws.sort_by(|a, b| a.0.cmp(&b.0));
    if !ws.is_empty() {
        blocks.push(vec![head("■ done / idle", "1")]);
        for (k, rs) in &ws {
            let name = v
                .ws_names
                .get(k)
                .cloned()
                .unwrap_or_else(|| format!("workspace {}", k.1));
            let parts = wrap(
                &format!("{name}  {}", tally(rs)),
                (inner - 1).max(2) as usize,
                (inner - 1).max(2) as usize,
            );
            let mut b = vec![format!(
                "  {}",
                paint(parts.first().map_or("", String::as_str), "2")
            )];
            b.extend(
                parts
                    .iter()
                    .skip(1)
                    .map(|c| format!("    {}", paint(c, "2"))),
            );
            blocks.push(b);
        }
    }
    (fit(&blocks, v.height, keep), order, chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ts: i64, kind: Kind) -> Event {
        Event {
            ts,
            src: 0,
            seq: 0,
            kind,
            body: format!("本文{ts}"),
        }
    }

    #[test]
    fn timeline_groups_and_counts_rest() {
        let mut es: Vec<Event> = (0..20)
            .map(|i| {
                ev(
                    1000 - i,
                    if i % 2 == 0 {
                        Kind::Instr
                    } else {
                        Kind::Report
                    },
                )
            })
            .collect();
        es.insert(1, ev(999, Kind::Instr));
        let lines = timeline_lines(&es, 80, false, 0, 1000, &|_| 0);
        assert_eq!(lines.len(), TL_MAX + 1, "最大 8 行と「他 N 件」のはず");
        assert!(
            lines[0].contains("×2 本文1000"),
            "隣り合う同じ種類は ×N にまとまるはず: {}",
            lines[0]
        );
        let shown: usize = group(&es)[..TL_MAX].iter().map(|g| g.1).sum();
        assert!(
            lines[TL_MAX].contains(&format!("他 {} 件", es.len() - shown)),
            "残りの件数が合うはず: {}",
            lines[TL_MAX]
        );
    }

    #[test]
    fn step_line_marks_done_and_unknown() {
        let d = Goal {
            steps: vec!["調査".into(), "対応表".into()],
            status: Some(Status::Reported {
                done: vec!["調査".into()],
                unknown: 2,
            }),
            ..Default::default()
        };
        assert_eq!(
            step_line(&d, 200).as_deref(),
            Some("step 1/2  ✓調査 ・対応表  (brief に無い step 2 件)"),
            "済みと未済と brief に無い件数を出すはず"
        );
        assert!(
            step_line(&Goal::default(), 200).is_none(),
            "steps が無ければ出さないはず"
        );
    }
}
