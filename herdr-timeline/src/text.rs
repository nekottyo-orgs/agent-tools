// 表示幅・折り返し・外部入力の整形・時刻。regex crate を足さないため、試作の正規表現は手で書き下す。

use crate::core::Kind;

// East Asian Width が W / F の主な範囲。▲ ● ■ … などの A (曖昧) は 1 桁として扱う (試作の unicodedata と同じ)
const WIDE: &[(u32, u32)] = &[
    (0x1100, 0x115F),
    (0x231A, 0x231B),
    (0x2329, 0x232A),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x2E80, 0x303E),
    (0x3041, 0x33FF),
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xA000, 0xA4CF),
    (0xA960, 0xA97F),
    (0xAC00, 0xD7A3),
    (0xF900, 0xFAFF),
    (0xFE10, 0xFE19),
    (0xFE30, 0xFE6F),
    (0xFF00, 0xFF60),
    (0xFFE0, 0xFFE6),
    (0x1F300, 0x1F64F),
    (0x1F900, 0x1F9FF),
    (0x20000, 0x2FFFD),
    (0x30000, 0x3FFFD),
];

pub fn wide(c: char) -> usize {
    let u = c as u32;
    if WIDE.iter().any(|&(a, b)| a <= u && u <= b) {
        2
    } else {
        1
    }
}

pub fn width_of(s: &str) -> usize {
    s.chars().map(wide).sum()
}

/// 表示幅 n 以内に切り詰める。超えたときは末尾を … にする
pub fn clip(s: &str, n: usize) -> String {
    if width_of(s) <= n {
        return s.to_string();
    }
    if n == 0 {
        return String::new();
    }
    let (mut out, mut w) = (String::new(), 0);
    for c in s.chars() {
        if w + wide(c) > n - 1 {
            break;
        }
        out.push(c);
        w += wide(c);
    }
    out + "…"
}

pub fn pad(s: &str, n: usize) -> String {
    let s = clip(s, n);
    let w = width_of(&s);
    s + &" ".repeat(n.saturating_sub(w))
}

// 行頭に落とさない文字と、行末に残さない文字
const CLOSE: &str = ")]}」』）】。、,.:;!?";
const OPEN: &str = "([{「『（【";
// 幅を超える語を切るとき、この文字の直後を優先する
const SOFT: &str = "_-/.";

/// 折り返しの単位。ASCII の非空白の連なりは 1 語、非 ASCII は 1 文字ずつ、空白の連なりは 1 つ
/// (列の区切りの 2 空白を保つため畳まない)
fn atoms_of(s: &str) -> Vec<String> {
    let (mut out, mut cur) = (Vec::new(), String::new());
    for c in s.chars() {
        if !c.is_ascii() && !c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(c.to_string());
        } else if cur
            .chars()
            .next()
            .is_some_and(|h| h.is_whitespace() != c.is_whitespace())
        {
            out.push(std::mem::replace(&mut cur, c.to_string()));
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn split_long(a: &str, n: usize) -> String {
    let (mut w, mut end, mut best) = (0, 0, 0);
    for (k, c) in a.char_indices() {
        if w + wide(c) > n {
            break;
        }
        w += wide(c);
        end = k + c.len_utf8();
        if SOFT.contains(c) {
            best = end;
        }
    }
    a[..if best > 0 { best } else { end }].to_string()
}

/// 表示幅で折り返した行。1 行目は幅 n1、2 行目以降は n2。ASCII の語の途中では折らず、
/// 閉じ括弧・句読点だけが行頭に、開き括弧だけが行末に残るときは直前の語ごと次の行へ送る
pub fn wrap(s: &str, n1: usize, n2: usize) -> Vec<String> {
    let mut atoms = atoms_of(s);
    let mut lines: Vec<String> = Vec::new();
    let mut cur: Vec<(String, String, usize)> = Vec::new();
    let (mut i, mut sp) = (0, String::new());
    let text = |cur: &[(String, String, usize)]| {
        cur.iter()
            .map(|(a, g, _)| format!("{g}{a}"))
            .collect::<String>()
    };
    while i < atoms.len() {
        let a = atoms[i].clone();
        if a.starts_with(char::is_whitespace) {
            sp = if cur.is_empty() { String::new() } else { a };
            i += 1;
            continue;
        }
        let n = (if lines.is_empty() { n1 } else { n2 }).max(2);
        if width_of(&text(&cur)) + width_of(&sp) + width_of(&a) <= n {
            cur.push((a, std::mem::take(&mut sp), i));
            i += 1;
        } else if cur.is_empty() {
            let cut = split_long(&a, n);
            atoms[i] = a[cut.len()..].to_string();
            lines.push(cut);
            if atoms[i].is_empty() {
                i += 1;
            }
        } else {
            let last_open = cur
                .last()
                .is_some_and(|(x, _, _)| x.chars().count() == 1 && OPEN.contains(x.as_str()));
            if cur.len() >= 2 && (a.starts_with(|c| CLOSE.contains(c)) || last_open) {
                i = cur.pop().map_or(i, |x| x.2);
            }
            lines.push(text(&cur));
            cur.clear();
            sp.clear();
        }
    }
    if !cur.is_empty() {
        lines.push(text(&cur));
    }
    lines
}

/// 行の塊を上から詰め、高さを超えるぶんは省略した行数を 1 行で出す。要対応が先頭なので、足りなくなっても
/// 要対応から見える。keep (選択行の塊の位置) が下に切れるなら、塊が入るまで表示範囲を下へずらす
pub fn fit(blocks: &[Vec<String>], height: Option<usize>, keep: Option<usize>) -> Vec<String> {
    let lines: Vec<String> = blocks.iter().flatten().cloned().collect();
    let Some(height) = height.filter(|h| lines.len() > *h) else {
        return lines;
    };
    let room = height.saturating_sub(1).max(1);
    let mut start = 0;
    if let Some(k) = keep {
        let top: usize = blocks[..k].iter().map(Vec::len).sum();
        start = (top + blocks[k].len().min(room)).saturating_sub(room);
    }
    let below = lines.len().saturating_sub(start + room);
    let cut = match (start, below) {
        (s, b) if s > 0 && b > 0 => format!("前 {s} 行と以降 {b} 行"),
        (s, _) if s > 0 => format!("前 {s} 行"),
        (_, b) => format!("以降 {b} 行"),
    };
    let mut out: Vec<String> = lines[start..(start + room).min(lines.len())].to_vec();
    out.push(format!("… {cut}を省略 (端末の高さが足りない)"));
    out
}

pub fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn first_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .map(squash)
        .unwrap_or_default()
}

/// 外部入力を端末に出す前の整形。CSI 列は丸ごと落とす (ESC だけ消すと "[31m" のような断片が残る)。
/// 残りの C0 / C1 制御文字と DEL は空白にして詰める
pub fn clean(s: &str) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '\x1b' && cs.get(i + 1) == Some(&'[') {
            let mut j = i + 2;
            while j < cs.len() && ('\x30'..='\x3f').contains(&cs[j]) {
                j += 1;
            }
            while j < cs.len() && ('\x20'..='\x2f').contains(&cs[j]) {
                j += 1;
            }
            if j < cs.len() && ('\x40'..='\x7e').contains(&cs[j]) {
                i = j + 1;
                continue;
            }
        }
        let u = c as u32;
        out.push(if u < 0x20 || (0x7f..=0x9f).contains(&u) {
            ' '
        } else {
            c
        });
        i += 1;
    }
    squash(&out)
}

pub const BODY_MAX: usize = 40;

/// Timeline の本文を短くする。[tag]・8 桁 id・「〜から: 」の前置きを外し、パスは basename にし、最初の文だけ残す
pub fn tl_body(kind: Kind, text: &str) -> String {
    let mut t = strip_tags(text);
    if kind != Kind::Msg {
        t = strip_from(t);
    }
    let t = basenames(t);
    // 「名前: (a) …」の括弧で切ると名前だけが残るので、":" で終わる切れ目は飛ばす
    let cut = t
        .char_indices()
        .filter(|&(i, c)| c == '。' || c == '（' || (c == '(' && t[..i].ends_with(' ')))
        .map(|(i, _)| i)
        .find(|&i| {
            let h = t[..i].trim();
            !h.is_empty() && !h.ends_with([':', '：'])
        })
        .unwrap_or(t.len());
    let head = t[..cut].trim();
    clip(if head.is_empty() { &t } else { head }, BODY_MAX)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn strip_tags(s: &str) -> &str {
    let mut t = s;
    while let Some(rest) = t.strip_prefix('[') {
        match rest.find(']') {
            Some(end) if end > 0 && rest[..end].chars().all(|c| is_word(c) || c == '-') => {
                t = rest[end + 1..].trim_start()
            }
            _ => break,
        }
    }
    let b = t.as_bytes();
    if b.len() >= 8
        && b[..8]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
        && !t[8..].starts_with(is_word)
    {
        t = t[8..].trim_start();
    }
    t
}

/// 「窓口から: 」の前置き。from 種類の「送り手: 本文」は別物なので呼び手が外さない
fn strip_from(s: &str) -> &str {
    for (idx, _) in s
        .match_indices("から")
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let pre = &s[..idx];
        if pre.is_empty() || pre.chars().count() > 20 || pre.contains(char::is_whitespace) {
            continue;
        }
        let after = &s[idx + "から".len()..];
        if let Some(r) = after.strip_prefix(':').or_else(|| after.strip_prefix('：')) {
            return r.trim_start();
        }
    }
    s
}

/// 絶対パス (/ か ~ で始まる語) は basename だけ読めれば足りる
fn basenames(s: &str) -> String {
    let (mut out, mut tok) = (String::new(), String::new());
    for c in s.chars() {
        if c.is_whitespace() {
            out += &basename(&tok);
            tok.clear();
            out.push(c);
        } else {
            tok.push(c);
        }
    }
    out + &basename(&tok)
}

fn basename(tok: &str) -> String {
    if !(tok.starts_with('/') || tok.starts_with('~')) {
        return tok.to_string();
    }
    let b = tok.as_bytes();
    match (1..b.len().saturating_sub(1)).rev().find(|&p| b[p] == b'/') {
        Some(p) => tok[p + 1..].to_string(),
        None => tok.to_string(),
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// ISO8601 (YYYY-MM-DDTHH:MM:SS[.fff][Z|±HH:MM]) を epoch 秒にする。offset が無ければ UTC とみなす
pub fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 19
        || b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let t = s.get(r)?;
        t.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| t.parse().ok())?
    };
    let (y, mo, d, h, mi, se) = (
        num(0..4)?,
        num(5..7)?,
        num(8..10)?,
        num(11..13)?,
        num(14..16)?,
        num(17..19)?,
    );
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let off = match rest {
        "" | "Z" => 0,
        _ => {
            let sign = match rest.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let r = &rest[1..];
            if r.len() != 5 || r.as_bytes()[2] != b':' {
                return None;
            }
            let hh: i64 = r[..2].parse().ok()?;
            let mm: i64 = r[3..].parse().ok()?;
            sign * (hh * 3600 + mm * 60)
        }
    };
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se - off)
}

/// 表示用の時刻。今日 (now と同じ日) なら HH:MM、違えば MM-DD HH:MM。off は UTC からのずれ (秒)。
/// TZ を引数にするのは、テストを実行環境の TZ に依存させないため
pub fn fmt_stamp(ts: i64, now: i64, off: &dyn Fn(i64) -> i64) -> String {
    let lt = ts + off(ts);
    let day = lt.div_euclid(86400);
    let sec = lt.rem_euclid(86400);
    let hm = format!("{:02}:{:02}", sec / 3600, sec % 3600 / 60);
    if day == (now + off(now)).div_euclid(86400) {
        hm
    } else {
        let (_, m, d) = civil_from_days(day);
        format!("{m:02}-{d:02} {hm}")
    }
}

pub fn age_text(sec: Option<i64>) -> String {
    match sec {
        Some(s) if s >= 0 => {
            for (unit, n) in [("d", 86400), ("h", 3600), ("m", 60)] {
                if s >= n {
                    return format!("{}{unit}", s / n);
                }
            }
            format!("{s}s")
        }
        _ => String::new(),
    }
}

fn brief_char(c: char) -> bool {
    is_word(c) || c == '.' || c == '-'
}

/// 指示の本文にある /<...>/brief-<name>.md の絶対パス (試作の `(/\S+/brief-[\w.-]+\.md)`)
pub fn brief_path(s: &str) -> Option<String> {
    s.split_whitespace().find_map(|tok| {
        tok.match_indices('/').find_map(|(start, _)| {
            // 語頭の / だけ。foo/brief-x.md の途中や URL の中は拾わない。
            // 日本語の句点や引用符の直後は語頭とみなすので、ASCII の語・パス・URL の文字の直後だけを外す
            if tok[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || "/:._-~%".contains(c))
            {
                return None;
            }
            let rest = &tok[start..];
            rest.match_indices("/brief-")
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .find_map(|(k, _)| {
                    if k < 2 {
                        return None;
                    }
                    let name = &rest[k + "/brief-".len()..];
                    let run = name.find(|c| !brief_char(c)).unwrap_or(name.len());
                    let end = name[..run]
                        .rmatch_indices(".md")
                        .map(|(i, _)| i)
                        .find(|&i| i > 0)?;
                    Some(rest[..k + "/brief-".len() + end + 3].to_string())
                })
        })
    })
}

pub const GOAL_MAX: usize = 60;

/// brief の最初の「# 」行。先頭の「依頼: 」は除く
pub fn goal_of(brief: &str) -> Option<String> {
    let h1 = brief.lines().find_map(|l| l.strip_prefix("# "))?;
    let h1 = h1
        .strip_prefix("依頼:")
        .or_else(|| h1.strip_prefix("依頼："))
        .unwrap_or(h1);
    let g = clean(h1);
    (!g.is_empty()).then(|| clip(&g, GOAL_MAX))
}

/// frontmatter の `steps: [a, b]` (flow 形式の配列だけ)
pub fn parse_steps(brief: &str) -> Vec<String> {
    let mut lines = brief.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Vec::new();
    }
    for l in lines {
        if l.trim_end() == "---" {
            break;
        }
        let Some(v) = l.strip_prefix("steps:") else {
            continue;
        };
        let Some(inner) = v.trim().strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
            return Vec::new();
        };
        return inner
            .split(',')
            .map(|x| clean(x.trim().trim_matches(|c| c == '"' || c == '\'')))
            .filter(|x| !x.is_empty())
            .collect();
    }
    Vec::new()
}

/// [progress] の本文。最初の空でない行の、先頭の [progress] を除いて , と 、 で分けたもの
pub fn progress_names(body: &str) -> Vec<String> {
    let line = first_line(body);
    let line = line.strip_prefix("[progress]").unwrap_or(&line);
    line.split([',', '、'])
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_and_clip() {
        assert_eq!(width_of("aあ한Ａ"), 7, "全角は 2 桁のはず");
        assert_eq!(width_of("▲●✓…"), 4, "曖昧幅の記号は 1 桁のはず");
        assert_eq!(clip("あいうえお", 5), "あい…", "幅を超えたら … で切るはず");
        assert_eq!(pad("ab", 4), "ab  ", "幅まで空白で埋めるはず");
    }

    #[test]
    fn wrap_keeps_words_and_punctuation() {
        assert_eq!(
            wrap("hello world foo", 11, 11),
            vec!["hello world", "foo"],
            "語の途中で折らないはず"
        );
        assert_eq!(
            wrap("あいう。", 6, 6),
            vec!["あい", "う。"],
            "句点だけを行頭に送らないはず"
        );
        assert_eq!(wrap("", 10, 10), Vec::<String>::new(), "空文字は空のはず");
        assert!(
            wrap("abcdefghij_klmnop", 12, 12)
                .iter()
                .all(|l| width_of(l) <= 12),
            "長い語は幅で切るはず"
        );
    }

    #[test]
    fn clean_drops_csi_and_controls() {
        assert_eq!(
            clean("\x1b[31m赤\x1b[0m  と\x07鳴る\x7f\u{9b}x"),
            "赤 と 鳴る x",
            "CSI は丸ごと落とし、制御文字は空白にして詰めるはず"
        );
        assert_eq!(clean("a\x1b]b"), "a ]b", "CSI でない ESC は空白になるはず");
    }

    #[test]
    fn tl_body_shortens() {
        assert_eq!(
            tl_body(
                Kind::Report,
                "[progress] 0a1b2c3d 窓口から: 表を作った。次は設計"
            ),
            "表を作った",
            "前置きを外し最初の文だけのはず"
        );
        assert_eq!(
            tl_body(Kind::Instr, "~/work/notes/plan.md を読んで"),
            "plan.md を読んで",
            "パスは basename のはず"
        );
        assert_eq!(
            tl_body(Kind::Msg, "調査役から: x"),
            "調査役から: x",
            "from は前置きを残すはず"
        );
        assert_eq!(
            tl_body(
                Kind::Report,
                "[progress] sample-task: (a) build が通った。次は PR"
            ),
            "sample-task: (a) build が通った",
            "名前の直後の括弧では切らないはず"
        );
    }

    #[test]
    fn time_parse_and_format() {
        let t = parse_time("2026-01-02T03:04:05.678Z").expect("読めるはず");
        assert_eq!(
            parse_time("2026-01-02T12:04:05+09:00"),
            Some(t),
            "offset を考慮するはず"
        );
        assert_eq!(parse_time("x"), None, "壊れた値は None のはず");
        let jst = |_: i64| 9 * 3600;
        assert_eq!(fmt_stamp(t, t + 60, &jst), "12:04", "同じ日は HH:MM のはず");
        assert_eq!(
            fmt_stamp(t, t + 86400, &jst),
            "01-02 12:04",
            "違う日は MM-DD HH:MM のはず"
        );
        assert_eq!(fmt_stamp(t, t, &|_| 0), "03:04", "UTC なら 03:04 のはず");
        assert_eq!(age_text(Some(3700)), "1h", "経過は大きい単位で丸めるはず");
    }

    #[test]
    fn brief_goal_steps() {
        assert_eq!(
            brief_path("これを読んで `/tmp/run/brief-a.b_c.md` 頼む").as_deref(),
            Some("/tmp/run/brief-a.b_c.md"),
            "brief の絶対パスを拾うはず"
        );
        assert_eq!(
            brief_path("foo/tmp/brief-x.md https://example.test/run/brief-y.md"),
            None,
            "語の途中や URL の中は拾わないはず"
        );
        assert_eq!(
            brief_path("/brief-x.md"),
            None,
            "ディレクトリの無いものは拾わないはず"
        );
        let b = "---\nsteps: [調査, \"対応表\", '設計案']\n---\n# 依頼: 表を作る\n";
        assert_eq!(
            parse_steps(b),
            vec!["調査", "対応表", "設計案"],
            "flow 形式の配列を読むはず"
        );
        assert_eq!(
            goal_of(b).as_deref(),
            Some("表を作る"),
            "H1 から依頼: を外すはず"
        );
        assert!(
            parse_steps("# x\nsteps: [a]").is_empty(),
            "frontmatter の外は読まないはず"
        );
        assert_eq!(
            progress_names("\n[progress] 調査、対応表 , x\n"),
            vec!["調査", "対応表", "x"],
            "区切りで分けるはず"
        );
    }
}
