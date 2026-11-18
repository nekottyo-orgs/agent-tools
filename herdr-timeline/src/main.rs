// 目的: herdr の pane を「要対応 / 作業中 / done・idle」の 3 段で一覧し、選んだ pane の Timeline
//       (人の指示・他 pane からのメッセージ・PR・inbox の報告) を時刻の降順で出す。
//       DeepSeek Harness (DSH) の Timeline を参考にした。
//
// 安全設計 (why):
//   - 読むだけ。書き込むのは herdr tab focus (Enter) だけで、pane には入力しない。サーバも立てない
//   - transcript の全体は展開した pane と --timeline の pane だけ読む (1 件のキャッシュ)。全 pane の行の
//     ゴールは、transcript が追記だけであることを使って前回の位置から後ろだけを読む (data::Briefs)。
//     working の transcript は周期ごとに変わるので、全体を読み直すと毎回すべてを解析し直すことになる
//   - 外部入力 (transcript・inbox・label・title) は端末に出す前に制御文字を落とす (text::clean)
//   - 外部コマンドやファイルの失敗では落とさず、警告 1 行か空にする
//
// 終了コード: 0 正常 (-h、--once・--timeline の描画、対話の終了)、1 --timeline の pane が無い、2 引数の誤り

mod core;
mod data;
mod text;
mod view;

use data::{
    canon, detail, gather, project_dirs, read_msgs, run_herdr, summary, transcript_mtime, Briefs,
    Detail, Env, PaneRow, Snapshot, Transcripts,
};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};
use view::{layout, paint, step_line, timeline_lines, Order, Row, View};

const USAGE: &str = "使い方: herdr-timeline [--once] [--interval N] [--width N] [--timeline PANE_ID] [--runs-dir DIR] [--show-resident] [-h|--help]";

const HELP: &str = "herdr の pane を 要対応 / 作業中 / done・idle の 3 段で一覧し、pane ごとの Timeline を出す。

使い方: herdr-timeline [--once] [--interval N] [--width N] [--timeline PANE_ID] [--runs-dir DIR] [--show-resident] [-h|--help]

オプション:
  --once             1 回描画して終わる
  --interval N       更新の間隔 (秒、既定 5、上限 86400)
  --width N          表示幅を固定する (既定は端末幅 - 1、最小 30、上限 1000)
  --timeline PANE_ID pane の Timeline を 1 回出して終わる (非対話。常駐の pane も引ける)
  --runs-dir DIR     run のディレクトリ (DIR/<run>/run.md と DIR/<run>/inbox/{new,cur}/*.msg)。
                     環境変数 HERDR_TIMELINE_RUNS_DIR でも渡せる。無ければ報告・進捗・未処理の報告を出さない
  --show-resident    常駐の pane (watch 系、Cue 専任、review-submit) と監視 skill が起こす PR review の pane は既定で一覧に出さない。これで出す
  -h, --help         この説明を出す

読み取り範囲:
  herdr の pane / tab / workspace の一覧、Claude Code の transcript (全 pane は追記ぶんだけ、Timeline は展開した pane の全体)、
  run の run.md と inbox の new・cur の *.msg、brief を読む。書き込まない
  (Enter の herdr tab focus を除く)。サーバを立てない。外部入力の制御文字は落としてから表示する

キー:
  ↑↓ / j k  要対応と作業中の行を選ぶ
  Enter     選んだ行の tab へ移動する (herdr tab focus)
  t         選んだ行の Timeline を開閉する
  q         終了する (Ctrl-C でも端末を戻して終わる)

環境変数:
  HERDR_TIMELINE_HERDR      herdr コマンド (既定 herdr、空白区切りで分割)
  HERDR_TIMELINE_HERDR_DIR  socket を探す場所 (既定 ~/.config/herdr)
  HERDR_TIMELINE_PROJECTS   transcript の projects ディレクトリ (既定 ~/.claude/projects)
  HERDR_TIMELINE_RUNS_DIR   --runs-dir と同じ
  HERDR_SOCKET_PATH         今の session の socket
  NO_COLOR / FORCE_COLOR    色を消す / tty でなくても色を付ける

終了コード:
  0  正常 (-h、--once・--timeline の描画、対話の終了)
  1  --timeline の pane が見つからない
  2  引数の誤り (--interval が 86400、--width が 1000 を超える場合を含む)
";

const INTERVAL_MAX: u64 = 86400;
const WIDTH_MAX: u64 = 1000;

struct Opts {
    once: bool,
    interval: u64,
    width: Option<usize>,
    timeline: Option<String>,
    runs: Option<PathBuf>,
    show_resident: bool,
}

/// Ok(None) は -h
fn parse_args(args: &[String]) -> Result<Option<Opts>, String> {
    let mut o = Opts {
        once: false,
        interval: 5,
        width: None,
        timeline: None,
        runs: None,
        show_resident: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || match it.next() {
            Some(v) if !v.starts_with('-') => Ok(v.clone()),
            Some(v) => Err(format!("{a} の値が - で始まる ({v})")),
            None => Err(format!("{a} に値が無い")),
        };
        let positive = |v: String| match v.parse::<u64>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err(format!("{a} は正の整数で指定する ({v})")),
        };
        // --interval は Instant への加算が overflow しない範囲、--width は 1 行の確保が膨らまない範囲に収める
        let bounded = |v: String, max: u64| match positive(v)? {
            n if n > max => Err(format!("{a} は {max} 以下で指定する ({n})")),
            n => Ok(n),
        };
        match a.as_str() {
            "-h" | "--help" => return Ok(None),
            "--once" => o.once = true,
            "--interval" => o.interval = bounded(value()?, INTERVAL_MAX)?,
            "--width" => o.width = Some(bounded(value()?, WIDTH_MAX)? as usize),
            "--timeline" => o.timeline = Some(value()?),
            "--runs-dir" => o.runs = Some(PathBuf::from(value()?)),
            "--show-resident" => o.show_resident = true,
            _ => return Err(format!("不明な引数 {a}")),
        }
    }
    Ok(Some(o))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// 表示側の TZ の UTC からのずれ (秒)
fn local_offset(t: i64) -> i64 {
    // SAFETY: tm は localtime_r が埋める出力先で、t はスタック上の値
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let t = t as libc::time_t;
        if libc::localtime_r(&t, &mut tm).is_null() {
            0
        } else {
            tm.tm_gmtoff as i64
        }
    }
}

fn isatty(fd: i32) -> bool {
    // SAFETY: fd を問い合わせるだけ
    unsafe { libc::isatty(fd) == 1 }
}

/// (列, 行)。環境変数 COLUMNS は親から古い値を引き継ぐことがあるので tty を直接問う
fn term_size() -> (usize, usize) {
    // SAFETY: winsize は ioctl が埋める出力先
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
            (ws.ws_col as usize, ws.ws_row as usize)
        } else {
            (80, 24)
        }
    }
}

/// 最終桁への描画は端末の自動折り返しで行が崩れることがあるので、指定が無いときは 1 桁狭く描く
fn term_width(fixed: Option<usize>) -> usize {
    fixed
        .unwrap_or_else(|| term_size().0.saturating_sub(1))
        .max(30)
}

/// 経過の起点。herdr は状態の開始時刻を返さないので、blocked は transcript の最終書き込み
/// (止まった時刻に近い)、working は viewer が working を見始めた時刻 (≥ を付ける)
struct Seen(HashMap<String, (String, i64)>);

impl Seen {
    fn rows(
        &mut self,
        snap: &Snapshot,
        now: i64,
        dirs: &[PathBuf],
        briefs: &mut Briefs,
    ) -> Vec<Row> {
        let progress = read_msgs(&snap.files, true);
        snap.panes
            .iter()
            .map(|p| {
                let e = self
                    .0
                    .entry(p.key.clone())
                    .or_insert((p.status.clone(), now));
                if e.0 != p.status {
                    *e = (p.status.clone(), now);
                }
                let working = p.status == "working";
                Row {
                    key: p.key.clone(),
                    tag: p.tag.clone(),
                    status: p.status.clone(),
                    name: p.name.clone(),
                    work: summary(&briefs.pane_goal(p, dirs, &progress), &p.title),
                    since: if working {
                        Some(e.1)
                    } else if p.status == "blocked" {
                        transcript_mtime(dirs, p.sid.as_deref())
                    } else {
                        None // 畳んだ行は経過を出さないので stat もしない
                    },
                    approx: working,
                    tab_id: p.tab_id.clone(),
                    ws: p.ws.clone(),
                }
            })
            .collect()
    }
}

/// 修飾なしの id は今の socket の session を優先し、無ければ他の session から探す
fn pick_pane<'a>(panes: &'a [PaneRow], pane: &str, own_sock: &str) -> Option<&'a PaneRow> {
    let own = canon(own_sock);
    let hit = |p: &&PaneRow| p.pane == pane || p.key == pane;
    panes
        .iter()
        .filter(hit)
        .find(|p| p.socket == own)
        .or_else(|| panes.iter().find(hit))
}

fn print_timeline(env: &Env, pane: &str, width: usize, color: bool) -> i32 {
    let snap = gather(env);
    let Some(row) = pick_pane(&snap.panes, pane, &env.own_sock.to_string_lossy()) else {
        let why = if snap.warn.is_empty() {
            String::new()
        } else {
            format!(" ({})", snap.warn.join(" / "))
        };
        eprintln!("pane {pane} が見つからない{why}");
        return 1;
    };
    let d = detail(
        &snap,
        row,
        &mut Transcripts::default(),
        &mut Briefs::default(),
        &project_dirs(&env.projects),
    );
    if d.events.is_empty() {
        let why = if d.has_transcript {
            "イベントが無い"
        } else {
            "transcript が見つからない"
        };
        println!("pane {pane} の Timeline が無い ({why})");
        return 0;
    }
    if let Some(g) = &d.goal.goal {
        println!("ゴール  {g}");
    }
    if let Some(s) = step_line(&d.goal, width) {
        println!("{s}");
    }
    for l in timeline_lines(&d.events, width, color, 0, now(), &local_offset) {
        println!("{l}");
    }
    0
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

/// 端末の設定を戻す。panic や早い return でも戻るよう Drop で行う
struct Term(Option<libc::termios>);

impl Drop for Term {
    fn drop(&mut self) {
        if let Some(t) = &self.0 {
            // SAFETY: 起動時に tcgetattr で得た設定を戻すだけ
            unsafe { libc::tcsetattr(0, libc::TCSADRAIN, t) };
        }
        print!("\x1b[?25h\x1b[?1049l");
        let _ = std::io::stdout().flush();
    }
}

/// 端末のキー入力。矢印は ESC で始まる列で届き read が途中で切れることもあるので、ESC 後の状態を
/// 読み取りをまたいで持つ (タイムアウトで ESC 単独を見分けない)。列の途中の文字は j / k / Enter として扱わない
#[derive(Default)]
struct KeyParser(u8); // 0 = 通常、1 = ESC の後、2 = CSI、3 = SS3

impl KeyParser {
    fn feed(&mut self, data: &[u8]) -> Vec<&'static str> {
        let mut out = Vec::new();
        for &b in data {
            match self.0 {
                0 => match b {
                    0x1b => self.0 = 1,
                    b'\r' | b'\n' => out.push("enter"),
                    b'k' => out.push("up"),
                    b'j' => out.push("down"),
                    b't' => out.push("t"),
                    b'q' => out.push("q"),
                    _ => {}
                },
                1 => {
                    self.0 = match b {
                        b'[' => 2,
                        b'O' => 3,
                        0x1b => 1,
                        _ => 0,
                    }
                }
                2 if (0x20..=0x3f).contains(&b) => {} // parameter と intermediate は読み捨てて final を待つ
                _ => {
                    self.0 = 0;
                    match b {
                        b'A' => out.push("up"),
                        b'B' => out.push("down"),
                        _ => {}
                    }
                }
            }
        }
        out
    }
}

fn focus_tab(env: &Env, tab: &str) -> String {
    match run_herdr(env, &["tab", "focus", tab], None) {
        Ok(_) => String::new(),
        Err(e) => format!("移動できない ({e})"),
    }
}

/// 展開中の pane の Timeline。キャッシュが効くので周期ごとに作り直してよい
fn expanded(
    snap: &Snapshot,
    key: &Option<String>,
    cache: &mut Transcripts,
    briefs: &mut Briefs,
    dirs: &[PathBuf],
) -> Option<(String, Detail)> {
    let k = key.as_ref()?;
    let p = snap.panes.iter().find(|p| &p.key == k)?;
    Some((k.clone(), detail(snap, p, cache, briefs, dirs)))
}

fn interactive(env: &Env, o: &Opts, color: bool) {
    for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: handler は AtomicBool を立てるだけで async-signal-safe
        unsafe { libc::signal(sig, on_signal as *const () as libc::sighandler_t) };
    }
    let tty = isatty(0);
    let mut saved = None;
    if tty {
        // SAFETY: stdin の termios を読み書きするだけ。ISIG は残すので Ctrl-C は signal で届く
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut t) == 0 {
                saved = Some(t);
                t.c_lflag &= !(libc::ICANON | libc::ECHO);
                t.c_cc[libc::VMIN] = 1;
                t.c_cc[libc::VTIME] = 0;
                libc::tcsetattr(0, libc::TCSADRAIN, &t);
            }
        }
    }
    let _term = Term(saved);
    print!("\x1b[?1049h\x1b[?25l");
    let mut seen = Seen(HashMap::new());
    let mut cache = Transcripts::default();
    let mut briefs = Briefs::default();
    let mut sel: Option<String> = None;
    let mut prev: Order = Vec::new();
    let mut parser = KeyParser::default();
    let mut tl_key: Option<String> = None;
    let mut notice = String::new();
    let mut stdout = std::io::stdout();
    while !STOP.load(Ordering::SeqCst) {
        // キー操作では取り直さず、保存した結果で配置だけやり直す
        let snap = gather(env);
        let dirs = project_dirs(&env.projects);
        let rows = seen.rows(&snap, now(), &dirs, &mut briefs);
        let mut ex = expanded(&snap, &tl_key, &mut cache, &mut briefs, &dirs);
        let deadline = Instant::now() + Duration::from_secs(o.interval);
        let mut redraw = true;
        loop {
            if STOP.load(Ordering::SeqCst) {
                return;
            }
            if redraw {
                // 幅も高さも描画のたびに取り直す。pane の大きさを変えたら次の描画から追従する
                let height = term_size().1.saturating_sub(1).max(2);
                let v = View {
                    rows: &rows,
                    runs: &snap.runs,
                    warn: &snap.warn,
                    names: &snap.names,
                    ws_names: &snap.ws_names,
                    width: term_width(o.width),
                    color,
                    select: tty.then_some((sel.clone(), &prev)),
                    height: Some(height - usize::from(tty)),
                    now: now(),
                    expanded: ex.as_ref().map(|(k, d)| (k.as_str(), d)),
                    off: &local_offset,
                };
                let (mut lines, order, chosen) = layout(&v);
                prev = order;
                sel = chosen;
                if tty {
                    let hint = "↑↓ / j k: 選択   Enter: tab へ移動   t: Timeline   q: 終了";
                    lines.push(paint(
                        color,
                        if notice.is_empty() { hint } else { &notice },
                        "2",
                    ));
                }
                lines.truncate(height);
                let body: String = lines.iter().map(|l| format!("{l}\x1b[K\n")).collect();
                let _ = write!(stdout, "\x1b[H{body}\x1b[J");
                let _ = stdout.flush();
                notice.clear();
                redraw = false;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            // signal の flag を見るため、待ちは 200ms ずつに区切る
            let wait = left.min(Duration::from_millis(200)).as_millis() as i32;
            if !tty {
                std::thread::sleep(Duration::from_millis(wait as u64));
                continue;
            }
            let mut pfd = libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: 1 要素の pollfd を渡すだけ
            if unsafe { libc::poll(&mut pfd, 1, wait) } <= 0 {
                continue;
            }
            let mut buf = [0u8; 64];
            // SAFETY: buf の長さまでしか書かれない
            let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                std::thread::sleep(Duration::from_millis(wait as u64)); // EOF で poll が即返り続けるのを避ける
                continue;
            }
            let ids: Vec<String> = prev.iter().map(|x| x.0.clone()).collect();
            for k in parser.feed(&buf[..n as usize]) {
                match k {
                    "q" => return,
                    "enter" => {
                        // tab_id が空の行 (別 session) は今の session から移れないので、移らずに理由を出す
                        match sel.as_ref().and_then(|s| prev.iter().find(|x| &x.0 == s)) {
                            Some(x) if x.1.is_empty() => {
                                notice = "別 session の行の tab には移れない".to_string()
                            }
                            Some(x) => notice = focus_tab(env, &x.1),
                            None => {}
                        }
                    }
                    "t" if sel.is_some() => {
                        tl_key = if tl_key == sel { None } else { sel.clone() };
                        ex = expanded(&snap, &tl_key, &mut cache, &mut briefs, &dirs);
                    }
                    "up" | "down" => {
                        if let Some(i) = sel.as_ref().and_then(|s| ids.iter().position(|x| x == s))
                        {
                            let j = if k == "up" {
                                i.saturating_sub(1)
                            } else {
                                (i + 1).min(ids.len() - 1)
                            };
                            sel = Some(ids[j].clone());
                        }
                    }
                    _ => {}
                }
                redraw = true;
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = match parse_args(&args) {
        Ok(Some(o)) => o,
        Ok(None) => {
            print!("{HELP}");
            return;
        }
        Err(e) => {
            eprintln!("herdr-timeline: {e}\n{USAGE}");
            exit(2);
        }
    };
    let env = Env::from_env(o.runs.clone(), o.show_resident || o.timeline.is_some());
    let color = std::env::var_os("NO_COLOR").is_none()
        && (isatty(1) || std::env::var_os("FORCE_COLOR").is_some());
    if let Some(p) = &o.timeline {
        exit(print_timeline(&env, p, term_width(o.width), color));
    }
    if !o.once {
        interactive(&env, &o, color);
        return;
    }
    let snap = gather(&env);
    let t = now();
    let rows = Seen(HashMap::new()).rows(
        &snap,
        t,
        &project_dirs(&env.projects),
        &mut Briefs::default(),
    );
    let v = View {
        rows: &rows,
        runs: &snap.runs,
        warn: &snap.warn,
        names: &snap.names,
        ws_names: &snap.ws_names,
        width: term_width(o.width),
        color,
        select: None,
        height: None,
        now: t,
        expanded: None,
        off: &local_offset,
    };
    for l in layout(&v).0 {
        println!("{l}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane_row(key: &str, pane: &str, socket: &str) -> PaneRow {
        PaneRow {
            key: key.into(),
            pane: pane.into(),
            tag: String::new(),
            status: "working".into(),
            name: String::new(),
            title: String::new(),
            sid: None,
            tab_id: None,
            ws: (String::new(), String::new()),
            socket: socket.into(),
        }
    }

    #[test]
    fn req6_interval_over_limit_is_rejected() {
        let a = |v: &str| parse_args(&["--interval".into(), v.into()]);
        assert!(a("86400").is_ok(), "上限ちょうどは通るはず");
        assert!(a("86401").is_err(), "上限超過は誤りのはず");
        assert!(
            a("18446744073709551615").is_err(),
            "u64 の最大でも panic せず誤りのはず"
        );
    }

    #[test]
    fn unqualified_pane_prefers_own_socket() {
        let panes = [
            pane_row("alpha/wA:p1", "wA:p1", "/nonexistent/alpha.sock"),
            pane_row("beta/wA:p1", "wA:p1", "/nonexistent/beta.sock"),
        ];
        let p = pick_pane(&panes, "wA:p1", "/nonexistent/beta.sock").expect("見つかるはず");
        assert_eq!(p.key, "beta/wA:p1", "今の socket の session を選ぶはず");
        let p = pick_pane(&panes, "wA:p1", "/nonexistent/other.sock").expect("見つかるはず");
        assert_eq!(p.key, "alpha/wA:p1", "今の socket に無ければ先頭を選ぶはず");
        let p = pick_pane(&panes, "alpha/wA:p1", "/nonexistent/beta.sock").expect("見つかるはず");
        assert_eq!(
            p.key, "alpha/wA:p1",
            "完全な key なら別 session も選べるはず"
        );
    }

    #[test]
    fn key_parser_keeps_state_across_reads() {
        let mut p = KeyParser::default();
        assert_eq!(
            p.feed(b"\x1b["),
            Vec::<&str>::new(),
            "列の途中では何も出さないはず"
        );
        assert_eq!(
            p.feed(b"Bj"),
            vec!["down", "down"],
            "読み取りをまたいだ矢印を解釈するはず"
        );
        assert_eq!(
            p.feed(b"\x1b[1;5Aq\r"),
            vec!["up", "q", "enter"],
            "修飾付きの矢印と通常のキーを解釈するはず"
        );
        assert_eq!(
            p.feed(b"\x1bj"),
            Vec::<&str>::new(),
            "ESC の直後の文字は Alt+キーとして無視するはず"
        );
    }
}
