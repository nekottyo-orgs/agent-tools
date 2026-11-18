// 画面の組み立て。描画する文字列を返すだけで、端末には触らない (試作の layout と同じ見た目を目標にする)。

use crate::core::{group, total, Event, Kind, Status};
use crate::data::{Detail, Goal, Run};
use crate::text::{age_text, clip, clip_head, fit, fmt_stamp, pad, tl_body, width_of, wrap};
use std::collections::HashMap;

const NAME_MAX: usize = 44;
const WORK_MIN: usize = 8; // 要約に最低限必要な幅。足りない行は要約を省く
const PANE_ID_MAX: usize = 20; // 行末の pane id の列幅の上限。別 session の行は session 名が付く
const AGE_W: usize = 4;
const ALERT_STYLE: &str = "1;31";
pub const TL_MAX: usize = 8; // Timeline に出す行数の上限 (まとめた後)。残りは「他 N 件」
/// 並べたときに Timeline に最低限残す桁数。これに満たない描画幅では積む
const TL_MIN_W: usize = 20;

pub const SIDE_MIN_WIDTH: usize = 120; // この幅以上なら一覧と Timeline を左右に並べ、未満なら上下に積む
pub const LIST_W: usize = 70; // 並べるときの一覧の幅。Timeline は残り
const SEP: &str = " │ "; // 並べるときの区切り (縦線 1 桁と左右の余白)

pub struct Row {
    pub key: String,
    pub tag: String,
    pub status: String,
    pub name: String,
    pub work: String,
    pub since: Option<i64>,
    /// 経過が観測起点 (viewer が状態を見始めた時刻。transcript が読めない、または turn が無いときだけ) なら真。≥ を付ける
    pub approx: bool,
    pub tab_id: Option<String>,
    pub ws: (String, String),
}

/// (key, tab_id) の並び。表示順で、選択の対象になる行。tab_id が空なのは tab に移れない (別 session の) 行
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
}

pub fn paint(color: bool, s: &str, code: &str) -> String {
    if color && !code.is_empty() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

// herdr 本体の status_indicators = "symbols" (src/client/shell.rs の status_icon、v0.9.5) と同じ記号。
// 既定の "dots" は blocked / working / done がどれも ● で、色が出ない端末では区別できないので symbols 側に揃える
fn symbol(status: &str) -> &'static str {
    match status {
        "blocked" => "×",
        "working" => "◐",
        "idle" => "○",
        "done" => "✓",
        _ => "·",
    }
}

// blocked の行は行全体を赤太字で塗る (whole) ので記号にはこの色を使わない。ここに残すのは status と色の対応を 1 か所に揃えるため
// herdr 本体の status_color (同ファイル) と同じ対応。palette の yellow / red / teal / green / overlay0 を ANSI の 33 / 31 / 36 / 32 / dim に当てる
fn status_style(status: &str) -> &'static str {
    match status {
        "blocked" => "31",
        "working" => "33",
        "idle" => "32",
        "done" => "36",
        _ => "2",
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

/// 一覧と Timeline の配置
#[derive(Debug, PartialEq)]
pub enum Split {
    Side { list_w: usize, tl_w: usize },
    Stack { list_h: usize, tl_h: usize },
}

/// 端末の列数 (phys) で並べる / 積むを決め、各幅は描画幅 width (列数 - 1) から取る。
/// 積むときの高さは一覧 6 : Timeline 4 で、どちらも 3 行は確保する
pub fn split(phys: usize, width: usize, height: usize) -> Split {
    // phys と width は別々に読まれて食い違うことがあるので、幅が足りなければ積む
    if phys >= SIDE_MIN_WIDTH && width >= LIST_W + width_of(SEP) + TL_MIN_W {
        return Split::Side {
            list_w: LIST_W,
            tl_w: width.saturating_sub(LIST_W + width_of(SEP)),
        };
    }
    let tl_h = (height * 4 / 10).max(3).min(height.saturating_sub(1));
    Split::Stack {
        list_h: height - tl_h,
        tl_h,
    }
}

/// 色の列 (CSI ... m) を除いた表示幅。色付きの行を揃えるため
fn visible_width(s: &str) -> usize {
    let (mut w, mut esc) = (0, false);
    for c in s.chars() {
        match (esc, c) {
            (_, '\x1b') => esc = true,
            (true, 'm') => esc = false,
            (false, c) => w += crate::text::wide(c),
            _ => {}
        }
    }
    w
}

/// 選択行の Timeline の欄。None は選択なし
pub fn timeline_panel(
    d: Option<&Detail>,
    width: usize,
    color: bool,
    now: i64,
    off: &dyn Fn(i64) -> i64,
) -> Vec<String> {
    let Some(d) = d else {
        return vec![paint(color, "選択なし", "2")];
    };
    let mut out = vec![paint(color, "■ Timeline", "1")];
    out.extend(step_line(&d.goal, width).map(|s| paint(color, &s, "2")));
    if d.events.is_empty() {
        out.push(paint(color, "イベントが無い", "2"));
    }
    out.extend(timeline_lines(&d.events, width, color, 0, now, off));
    out
}

/// 行数が h を超えるなら末尾を 1 行の省略表示に替える。欠けていることが見えるようにするため
fn fit_rows(mut rows: Vec<String>, h: usize) -> Vec<String> {
    if rows.len() <= h {
        return rows;
    }
    if h == 0 {
        return Vec::new();
    }
    let hidden = rows.len() - (h - 1);
    rows.truncate(h - 1);
    rows.push(format!("他 {hidden} 件"));
    rows
}

/// 一覧と Timeline を 1 つの画面にする。どの行も height 行・width 桁を超えない
pub fn compose(sp: &Split, list: Vec<String>, tl: Vec<String>, height: usize) -> Vec<String> {
    match *sp {
        Split::Side { list_w, tl_w } => {
            let tl = fit_rows(tl, height);
            (0..list.len().max(tl.len()).min(height))
                .map(|i| {
                    let l = list.get(i).map_or("", String::as_str);
                    let r = tl.get(i).map_or("", String::as_str);
                    let gap = list_w.saturating_sub(visible_width(l));
                    debug_assert!(visible_width(r) <= tl_w);
                    format!("{l}{}{SEP}{r}", " ".repeat(gap))
                })
                .collect()
        }
        Split::Stack { list_h, tl_h } => list
            .into_iter()
            .take(list_h)
            .chain(fit_rows(tl, tl_h))
            .collect(),
    }
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

/// tally の行を薄く塗り、tally の語のうち記号だけを状態の色に戻す。
/// wrap は空白で折るので tally は常に全行の末尾 W 語になる。語の位置で決めるので、名前が折り返されて同じ文字列が出ても塗らない
fn tint_tally(rows: &[&Row], parts: &[String], color: bool) -> Vec<String> {
    let total: usize = parts.iter().map(|p| p.split_whitespace().count()).sum();
    let start = total.saturating_sub(tally(rows).split_whitespace().count());
    let mut idx = 0;
    parts
        .iter()
        .map(|part| {
            let toks: Vec<String> = part
                .split(' ')
                .map(|t| {
                    if t.is_empty() {
                        return t.to_string();
                    }
                    idx += 1;
                    let k = ["done", "idle", "unknown"]
                        .into_iter()
                        .find(|k| symbol(k) == t && rows.iter().any(|x| x.status == *k));
                    match k {
                        Some(k) if color && idx > start => {
                            format!("\x1b[0m{}\x1b[2m", paint(true, symbol(k), status_style(k)))
                        }
                        _ => t.to_string(),
                    }
                })
                .collect();
            paint(color, &toks.join(" "), "2")
        })
        .collect()
}

fn tally(rows: &[&Row]) -> String {
    let c = |s: &str| rows.iter().filter(|x| x.status == s).count();
    ["done", "idle", "unknown"]
        .iter()
        .filter(|k| c(k) > 0)
        .map(|k| format!("{} {}", symbol(k), c(k)))
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
        .map(|x| (x.key.clone(), x.tab_id.clone().unwrap_or_default()))
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
                // whole の行は行全体が赤太字なので、記号だけを塗ると途中で色が切れる
                let sym = if whole {
                    symbol(&x.status).to_string()
                } else {
                    paint(symbol(&x.status), status_style(&x.status))
                };
                format!("{key}{sym} ")
            } else {
                " ".repeat(key_w as usize + 2)
            };
            let ag = if first { age(x) } else { " ".repeat(AGE_W) };
            if whole {
                let mut body = format!("{lead}{nm} {ag}{mid}");
                if first {
                    body += &format!(" {}", clip_head(&x.tag, pw as usize));
                }
                lines.push(blank.clone() + &paint(body.trim_end(), ALERT_STYLE));
            } else {
                let tag = if first {
                    paint(&format!(" {}", clip_head(&x.tag, pw as usize)), "2")
                } else {
                    String::new()
                };
                lines.push(format!("{blank}{lead}{} {ag}{mid}{tag}", paint(&nm, "1")));
            }
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
            let b = tint_tally(rs, &parts, v.color)
                .into_iter()
                .enumerate()
                .map(|(n, l)| format!("{}{l}", if n == 0 { "  " } else { "    " }))
                .collect::<Vec<_>>();
            blocks.push(b);
        }
    }
    (fit(&blocks, v.height, keep), order, chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_timeline_columns_and_follows_physical_width() {
        // 描画幅の上書きが狭くて Timeline が残らないなら積む
        let narrow = LIST_W + width_of(SEP) + 3;
        assert!(matches!(split(150, narrow, 30), Split::Stack { .. }));
        // 物理の列数が広ければ、描画幅が足りる限り並べる
        let ok = LIST_W + width_of(SEP) + TL_MIN_W;
        assert!(matches!(split(150, ok, 30), Split::Side { .. }));
    }

    #[test]
    fn truncated_timeline_shows_omitted_count() {
        let rows: Vec<String> = (0..9).map(|i| format!("e{i}")).collect();
        let out = fit_rows(rows.clone(), 4);
        assert_eq!(out.len(), 4, "高さを超えないはず");
        assert_eq!(out[3], "他 6 件", "欠けた行数が見えるはず");
        assert_eq!(fit_rows(rows.clone(), 9), rows, "収まるなら変えないはず");
    }

    fn ev(ts: i64, kind: Kind) -> Event {
        Event {
            ts,
            src: 0,
            seq: 0,
            kind,
            body: format!("本文{ts}"),
        }
    }

    fn row(key: &str, status: &str, tag: &str, tab: Option<&str>) -> Row {
        Row {
            key: key.into(),
            tag: tag.into(),
            status: status.into(),
            name: "sample".into(),
            work: "作業".into(),
            since: Some(0),
            approx: false,
            tab_id: tab.map(Into::into),
            ws: (String::new(), String::new()),
        }
    }

    fn detail_of(n: usize) -> Detail {
        Detail {
            has_transcript: true,
            events: (0..n).map(|i| ev(1000 - i as i64, Kind::Instr)).collect(),
            goal: Goal::default(),
        }
    }

    #[test]
    fn split_switches_at_side_min_width() {
        assert!(
            matches!(split(119, 118, 30), Split::Stack { .. }),
            "119 は積むはず"
        );
        assert_eq!(
            split(120, 119, 30),
            Split::Side {
                list_w: LIST_W,
                tl_w: 119 - LIST_W - 3
            },
            "120 は並べ、区切りの 3 桁を引くはず"
        );
    }

    #[test]
    fn split_stacks_when_width_is_too_narrow() {
        assert!(
            matches!(split(120, 59, 30), Split::Stack { .. }),
            "phys と width が食い違っても panic せず積むはず"
        );
    }

    #[test]
    fn side_lines_fit_width_even_with_color_and_wide_chars() {
        let list = vec![paint(true, "日本語の一覧", "1"), "abc".to_string()];
        let tl = timeline_panel(Some(&detail_of(3)), 47, true, 1000, &|_| 0);
        let sp = split(120, 119, 30);
        let out = compose(&sp, list, tl, 30);
        assert!(
            out.iter().all(|l| visible_width(l) <= 119),
            "どの行も端末幅を超えないはず: {out:?}"
        );
        assert!(out[0].contains(" │ "), "区切りが出るはず: {out:?}");
    }

    #[test]
    fn stack_lines_fit_height() {
        for h in [2, 5, 10, 24, 40] {
            let sp = split(80, 79, h);
            let list: Vec<String> = (0..100).map(|i| format!("l{i}")).collect();
            let tl = timeline_panel(Some(&detail_of(20)), 80, false, 1000, &|_| 0);
            assert!(
                compose(&sp, list, tl, h).len() <= h,
                "総行数が高さ {h} を超えないはず"
            );
        }
    }

    #[test]
    fn timeline_panel_follows_selection_and_handles_none() {
        let a = timeline_panel(Some(&detail_of(1)), 80, false, 1000, &|_| 0).join("\n");
        let b = timeline_panel(Some(&detail_of(2)), 80, false, 1000, &|_| 0).join("\n");
        assert_ne!(a, b, "選択が変わると中身も変わるはず");
        assert_eq!(
            timeline_panel(None, 80, false, 0, &|_| 0),
            vec!["選択なし"],
            "選択なしは 1 行のはず"
        );
    }

    #[test]
    fn foreign_session_rows_are_selectable_but_have_no_tab() {
        let rows = [
            row("a", "working", "p1", Some("t1")),
            row("b/p2", "working", "b/p2", None),
        ];
        let (names, ws) = (HashMap::new(), HashMap::new());
        let prev = Vec::new();
        let v = View {
            rows: &rows,
            runs: &[],
            warn: &[],
            names: &names,
            ws_names: &ws,
            width: 80,
            color: false,
            select: Some((None, &prev)),
            height: None,
            now: 10,
        };
        let (_, order, _) = layout(&v);
        assert_eq!(
            order,
            vec![("a".into(), "t1".into()), ("b/p2".into(), String::new())],
            "別 session の行も選択でき、tab_id は空のはず"
        );
    }

    #[test]
    fn long_pane_tag_is_clipped_to_reserved_width() {
        let (t1, t2) = (
            "very-long-session-name-for-test/wA:p1",
            "very-long-session-name-for-test/wA:p2",
        );
        let rows = [row("a", "working", t1, None), row("b", "blocked", t2, None)];
        let (names, ws) = (HashMap::new(), HashMap::new());
        let v = View {
            rows: &rows,
            runs: &[],
            warn: &[],
            names: &names,
            ws_names: &ws,
            width: 60,
            color: false,
            select: None,
            height: None,
            now: 10,
        };
        let (lines, _, _) = layout(&v);
        assert!(
            lines.iter().all(|l| width_of(l) <= 60),
            "どの行も表示幅を超えないはず: {lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains(t1) || l.contains(t2)),
            "tag は切り詰められるはず: {lines:?}"
        );
        for id in ["/wA:p1", "/wA:p2"] {
            assert!(
                lines.iter().any(|l| l.contains(id)),
                "切り詰めても末尾の pane id は残るはず ({id}): {lines:?}"
            );
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

    #[test]
    fn symbols_match_herdr_and_are_distinct_without_color() {
        let all = ["blocked", "working", "done", "idle", "unknown"];
        let got: Vec<&str> = all.iter().map(|s| symbol(s)).collect();
        assert_eq!(
            got,
            ["×", "◐", "✓", "○", "·"],
            "herdr の symbols と同じ記号のはず"
        );
        let mut uniq = got.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(
            uniq.len(),
            all.len(),
            "色が無くても記号だけで区別できるはず"
        );
        assert_eq!(
            symbol("something-else"),
            "·",
            "未知の状態は unknown と同じ記号のはず"
        );
    }

    #[test]
    fn tally_uses_icons_not_words() {
        let rows = [
            row("a", "done", "p1", None),
            row("b", "idle", "p2", None),
            row("c", "idle", "p3", None),
            row("d", "unknown", "p4", None),
        ];
        let refs: Vec<&Row> = rows.iter().collect();
        assert_eq!(
            tally(&refs),
            "✓ 1  ○ 2  · 1",
            "文字でなく記号と件数を出すはず"
        );
        let tinted = tint_tally(&refs, &["ws  ✓ 1  ○ 2  · 1".to_string()], true).concat();
        assert!(
            tinted.contains("\x1b[36m✓\x1b[0m") && tinted.contains("\x1b[32m○\x1b[0m"),
            "色ありなら記号が状態の色になるはず: {tinted:?}"
        );
        assert!(
            !tint_tally(&refs, &["ws  ✓ 1".to_string()], false)
                .concat()
                .contains('\x1b'),
            "色なしなら制御文字を出さないはず"
        );
    }

    #[test]
    fn tint_tally_skips_same_text_in_workspace_name() {
        let rows = [
            row("a", "done", "p1", None),
            row("b", "unknown", "p2", None),
        ];
        let refs: Vec<&Row> = rows.iter().collect();
        let t = tint_tally(&refs, &["ws ✓ 1 · 10  ✓ 1  · 1".to_string()], true).concat();
        assert!(
            t.starts_with("\x1b[2mws ✓ 1 · 10  "),
            "名前側は塗らないはず: {t:?}"
        );
        assert!(
            t.contains("\x1b[36m✓\x1b[0m"),
            "末尾の tally は塗るはず: {t:?}"
        );
        assert_eq!(
            t.matches("\x1b[36m").count(),
            1,
            "塗るのは 1 か所のはず: {t:?}"
        );
    }

    #[test]
    fn tint_tally_skips_wrapped_name_continuation() {
        let rows = [
            row("a", "done", "p1", None),
            row("b", "unknown", "p2", None),
        ];
        let refs: Vec<&Row> = rows.iter().collect();
        let parts = ["ws ✓ 1".to_string(), "foo ✓ 1  ✓ 1  · 1".to_string()];
        let t = tint_tally(&refs, &parts, true);
        assert!(
            t[0].starts_with("\x1b[2mws ✓ 1") && t[1].starts_with("\x1b[2mfoo ✓ 1  "),
            "名前側の記号は塗らないはず: {t:?}"
        );
        assert_eq!(
            t.concat().matches("\x1b[36m").count(),
            1,
            "塗るのは tally の ✓ だけのはず: {t:?}"
        );
        assert!(t[1].contains("\x1b[0m"), "tally の · も塗るはず: {t:?}");
    }

    #[test]
    fn blocked_row_keeps_whole_line_red_and_widths_hold_with_color() {
        let rows = [
            row("a", "blocked", "p1", Some("t1")),
            row("b", "done", "p2", Some("t2")),
            row("c", "idle", "p3", Some("t3")),
        ];
        let (names, ws) = (HashMap::new(), HashMap::new());
        let v = View {
            rows: &rows,
            runs: &[],
            warn: &[],
            names: &names,
            ws_names: &ws,
            width: 40,
            color: true,
            select: None,
            height: None,
            now: 10,
        };
        let lines = layout(&v).0;
        let blocked = lines.iter().find(|l| l.contains('×')).expect("blocked 行");
        assert!(
            blocked.matches("\x1b[0m").count() == 1 && blocked.ends_with("\x1b[0m"),
            "行全体を 1 回で塗り、途中で色を切らないはず: {blocked:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("\x1b[36m✓\x1b[0m") && l.contains("\x1b[32m○\x1b[0m")),
            "畳んだ行は色ありで記号が塗られるはず: {lines:?}"
        );
        let strip = |s: &str| {
            let mut o = String::new();
            let mut esc = false;
            for c in s.chars() {
                match (esc, c) {
                    (_, '\x1b') => esc = true,
                    (true, 'm') => esc = false,
                    (false, c) => o.push(c),
                    _ => {}
                }
            }
            o
        };
        assert!(
            lines.iter().all(|l| width_of(&strip(l)) <= 40),
            "色を除いた幅が width 以下のはず: {lines:?}"
        );
    }

    #[test]
    fn working_row_symbol_is_colored_only_with_color() {
        let rows = [row("a", "working", "p1", Some("t1"))];
        let (names, ws) = (HashMap::new(), HashMap::new());
        let mk = |color| {
            let v = View {
                rows: &rows,
                runs: &[],
                warn: &[],
                names: &names,
                ws_names: &ws,
                width: 80,
                color,
                select: None,
                height: None,
                now: 10,
            };
            layout(&v).0.join("\n")
        };
        assert!(
            mk(true).contains("\x1b[33m◐\x1b[0m"),
            "作業中は黄色の ◐ のはず"
        );
        let plain = mk(false);
        assert!(
            plain.contains('◐') && !plain.contains('\x1b'),
            "色なしでも ◐ は出るはず: {plain:?}"
        );
    }
}
