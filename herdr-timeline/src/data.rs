// herdr・run・inbox・transcript・brief の読み取り。どれも読むだけで、書き込みは herdr tab focus (main 側) しかしない。
// 外部コマンドやファイルの失敗は落とさず、空か警告 1 行にする。

use crate::core::{
    self, attributed, last_brief, order, status, transcript_events, valid_sid, Class, Event, Kind,
    Line, Msg, Progress, Status,
};
use crate::text::{
    brief_path, clean, first_line, goal_of, parse_steps, parse_time, progress_names, squash,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

// herdr の pane の API には常駐の印が無い (run-inbox の --resident は起動時の env を渡すだけで pane list に出ない)。
// そのため常駐かどうかは pane の label の完全一致で決める。常駐の pane を増やすときはここに足す。
pub const RESIDENT_LABELS: &[&str] = &[
    "watch-my-pr-reviews",
    "watch-other-pr-reviews",
    "watch-worker",
    "review-submit",
    "Cue回答の監視",
];

pub fn is_resident(label: &str) -> bool {
    RESIDENT_LABELS.contains(&label)
}

/// 監視 skill の dispatch が起こす PR review の pane の label (`<repo略称>-<PR番号>` / `<repo略称>-bundle-<PR番号>`)。
/// 形だけだと `feat-xxx-34` のような無関係な pane に当たり、run の台帳だけだと窓口や常駐が「run に属さない pane」として消える。
/// そのため呼び出し側で label の形と「run に属さない」の両方を満たすものだけを外す。
pub fn is_dispatched_label(label: &str) -> bool {
    let Some((head, num)) = label.rsplit_once('-') else {
        return false;
    };
    !head.is_empty() && !num.is_empty() && num.bytes().all(|b| b.is_ascii_digit())
}

/// pane の分類。常駐と dispatch の pane は既定の画面から外す (blocked を除く)
#[derive(Clone, PartialEq, Debug)]
pub enum PaneKind {
    Resident,
    /// `<repo略称>#<PR番号>`
    Dispatch(String),
}

/// dispatch の label から `<repo略称>#<番号>`。末尾の -bundle は repo 略称に含めない。is_dispatched_label が真の label だけを渡す
fn dispatch_ref(label: &str) -> String {
    let (head, num) = label.rsplit_once('-').unwrap_or((label, ""));
    format!("{}#{num}", head.strip_suffix("-bundle").unwrap_or(head))
}

pub struct Env {
    pub herdr: Vec<String>,
    pub herdr_dir: PathBuf,
    pub projects: PathBuf,
    pub codex_sessions: PathBuf,
    pub runs: Option<PathBuf>,
    pub own_sock: PathBuf,
    pub show_resident: bool,
    /// 真なら hidden な pane も snap.panes に残す (--json)。hidden の値は show_resident に依らない
    pub keep_hidden: bool,
}

impl Env {
    pub fn from_env(runs: Option<PathBuf>, show_resident: bool, keep_hidden: bool) -> Env {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let home = PathBuf::from(var("HOME").unwrap_or_default());
        let herdr_dir = var("HERDR_TIMELINE_HERDR_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config/herdr"));
        let own_sock = var("HERDR_SOCKET_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| herdr_dir.join("herdr.sock"));
        Env {
            herdr: var("HERDR_TIMELINE_HERDR")
                .unwrap_or_else(|| "herdr".into())
                .split_whitespace()
                .map(String::from)
                .collect(),
            herdr_dir,
            projects: var("HERDR_TIMELINE_PROJECTS")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude/projects")),
            codex_sessions: var("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"))
                .join("sessions"),
            runs: runs.or_else(|| var("HERDR_TIMELINE_RUNS_DIR").map(PathBuf::from)),
            own_sock,
            show_resident,
            keep_hidden,
        }
    }
}

/// socket の比較は両側を正規化してから。できなければ文字列のまま (旧 session の socket は消えていることがある)
pub fn canon(p: &str) -> String {
    fs::canonicalize(p)
        .map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_else(|_| p.to_string())
}

/// herdr を呼んで result を返す。std に timeout が無いので wait は別 thread に任せる。
/// stdout を読まずに待つと、出力がパイプの容量を超えたときに子が止まるため wait_with_output を使う
pub fn herdr(env: &Env, args: &[&str], sock: Option<&Path>) -> Result<Value, String> {
    let what = format!("herdr {}", args.join(" "));
    serde_json::from_slice::<Value>(&run_herdr(env, args, sock)?)
        .ok()
        .and_then(|v| v.get("result").cloned())
        .ok_or_else(|| format!("{what} の出力が JSON でない"))
}

/// herdr を 10 秒の timeout 付きで呼び、stdout を返す
pub fn run_herdr(env: &Env, args: &[&str], sock: Option<&Path>) -> Result<Vec<u8>, String> {
    let (prog, pre) = env.herdr.split_first().ok_or("HERDR_TIMELINE_HERDR が空")?;
    let mut cmd = Command::new(prog);
    cmd.args(pre)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(s) = sock {
        cmd.env("HERDR_SOCKET_PATH", s);
    }
    let what = format!("herdr {}", args.join(" "));
    let child = cmd
        .spawn()
        .map_err(|e| format!("{what} を起動できない ({e})"))?;
    let pid = child.id() as libc::pid_t;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(out)) if out.status.success() => Ok(out.stdout),
        Ok(Ok(out)) => Err(format!(
            "{what} が終了コード {} で失敗",
            out.status.code().unwrap_or(-1)
        )),
        Ok(Err(e)) => Err(format!("{what} を待てない ({e})")),
        Err(_) => {
            // SAFETY: 自分が起動した子の pid に signal を送るだけ
            unsafe { libc::kill(pid, libc::SIGKILL) };
            Err(format!("{what} が 10 秒で応答しない"))
        }
    }
}

/// [(session 名, socket)]。socket が 1 つ以下なら今の環境の 1 session として扱う。
/// ただし 1 つだけ見つかり、今の環境の socket が実在しないときは、見つかった方を使う (名前は付けず、text 表示の見た目は変えない)
fn sessions(env: &Env) -> Vec<(String, Option<PathBuf>)> {
    let mut found = vec![("default".to_string(), env.herdr_dir.join("herdr.sock"))];
    if let Ok(rd) = fs::read_dir(env.herdr_dir.join("sessions")) {
        let mut v: Vec<_> = rd
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    e.path().join("herdr.sock"),
                )
            })
            .collect();
        v.sort();
        found.extend(v);
    }
    found.retain(|(_, p)| p.exists());
    if found.len() > 1 {
        found.into_iter().map(|(n, p)| (n, Some(p))).collect()
    } else if let (Some((_, p)), false) = (found.first(), env.own_sock.exists()) {
        vec![(String::new(), Some(p.clone()))]
    } else {
        vec![(String::new(), None)]
    }
}

pub struct PaneRow {
    pub key: String,
    pub pane: String,
    pub tag: String,
    pub status: String,
    pub name: String,
    pub title: String,
    /// herdr の生の label。出力側で clean する
    pub label: String,
    pub kind: Option<PaneKind>,
    pub sid: Option<String>,
    pub codex: bool,
    pub tab_id: Option<String>,
    pub ws: (String, String),
    pub socket: String,
}

impl PaneRow {
    /// blocked は人の対応待ちなので、常駐・dispatch でも隠さない
    pub fn hidden(&self) -> bool {
        self.kind.is_some() && self.status != "blocked"
    }
}

pub enum RunWindow {
    Active(String),
    Absent,
    Unknown,
    Closed,
}

pub struct Run {
    pub id: String,
    pub window: RunWindow,
    pub orch: String,
    /// orchestrator_socket。無い旧形式の run は None (pane id だけで照合する)
    pub orch_socket: Option<String>,
    pub new: usize,
    pub oldest: Option<i64>,
}

pub struct InMsg {
    pub file: String,
    pub path: PathBuf,
    pub msg: Msg,
}

#[derive(Default)]
pub struct Snapshot {
    pub warn: Vec<String>,
    pub panes: Vec<PaneRow>,
    pub ws_names: HashMap<(String, String), String>,
    pub runs: Vec<Run>,
    /// inbox/new と inbox/cur の *.msg (ファイル名, パス)。中身は使う側が read_msgs で必要なものだけ読む
    pub files: Vec<(String, PathBuf)>,
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

pub fn status_of(s: &str) -> &'static str {
    match s {
        "blocked" => "blocked",
        "working" => "working",
        "idle" => "idle",
        "done" => "done",
        _ => "unknown",
    }
}

pub fn gather(env: &Env) -> Snapshot {
    let mut snap = Snapshot::default();
    let own = canon(&env.own_sock.to_string_lossy());
    let mut observed_sockets = HashSet::new();
    for (name, sock) in sessions(env) {
        let socket = sock
            .as_ref()
            .map_or_else(|| own.clone(), |p| canon(&p.to_string_lossy()));
        let foreign = !name.is_empty() && socket != own;
        let at = |e: String| {
            if name.is_empty() {
                e
            } else {
                format!("{name}: {e}")
            }
        };
        let panes = match herdr(env, &["pane", "list"], sock.as_deref()) {
            Ok(v) => v,
            Err(e) => {
                snap.warn.push(at(e));
                continue;
            }
        };
        // 空の一覧は不在を示せるが、壊れた一覧には窓口が含まれていたかもしれない。
        if panes
            .get("panes")
            .and_then(Value::as_array)
            .is_some_and(|ids| {
                ids.iter().all(|p| {
                    p.get("pane_id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| !clean(id).is_empty())
                })
            })
        {
            observed_sockets.insert(socket.clone());
        }
        let mut get = |what: &str| {
            herdr(env, &[what, "list"], sock.as_deref()).unwrap_or_else(|e| {
                snap.warn.push(at(e));
                Value::Null
            })
        };
        let (wl, tl) = (get("workspace"), get("tab"));
        let wl = arr(&wl, "workspaces");
        // 同じ label の workspace は画面で見分けがつかないので、Spaces 一覧の順番を足す
        let labels: Vec<(String, String, Option<u64>)> = wl
            .iter()
            .map(|w| {
                let id = s(w, "workspace_id");
                let l = clean(&s(w, "label"));
                (
                    id.clone(),
                    if l.is_empty() {
                        format!("workspace {id}")
                    } else {
                        l
                    },
                    w.get("number").and_then(Value::as_u64),
                )
            })
            .collect();
        let mut ws = HashMap::new();
        for (id, l, n) in &labels {
            let shown = match n {
                Some(n) if labels.iter().filter(|x| &x.1 == l).count() > 1 => {
                    format!("{l} ({n} 番目)")
                }
                _ => l.clone(),
            };
            snap.ws_names.insert(
                (name.clone(), id.clone()),
                if foreign {
                    format!("{name} / {shown}")
                } else {
                    shown.clone()
                },
            );
            ws.insert(id.clone(), shown);
        }
        let tabs: HashMap<String, &Value> = arr(&tl, "tabs")
            .iter()
            .map(|t| (s(t, "tab_id"), t))
            .collect();
        for p in arr(&panes, "panes") {
            let pane = clean(&s(p, "pane_id"));
            let tab = tabs.get(&s(p, "tab_id"));
            // pane の label は同じ tab に pane が複数あるときだけ足す (単独なら tab の label と同じ)
            let mut parts = vec![
                ws.get(&s(p, "workspace_id")).cloned().unwrap_or_default(),
                tab.map(|t| clean(&s(t, "label"))).unwrap_or_default(),
            ];
            if tab
                .and_then(|t| t.get("pane_count"))
                .and_then(Value::as_u64)
                .unwrap_or(1)
                > 1
            {
                parts.push(clean(&s(p, "label")));
            }
            let mut seen: Vec<String> = Vec::new();
            for x in parts {
                if !x.is_empty() && !seen.contains(&x) {
                    seen.push(x);
                }
            }
            let pname = seen.join(" / ");
            let tag = if name.is_empty() {
                pane.clone()
            } else {
                format!("{name}/{pane}")
            };
            let codex = p["agent"].as_str() == Some("codex");
            let sid = p
                .get("agent_session")
                .filter(|a| {
                    !codex
                        || (a["kind"].as_str() == Some("id")
                            && a["agent"].as_str() == Some("codex")
                            && a["source"].as_str() == Some("herdr:codex"))
                })
                .map(|a| s(a, "value"))
                .filter(|v| !v.is_empty());
            let label = s(p, "label");
            // dispatch は台帳 (run) を読んでから確定する。ここでは label の形だけで仮に付ける
            let kind = if is_resident(&label) {
                Some(PaneKind::Resident)
            } else if is_dispatched_label(&label) {
                Some(PaneKind::Dispatch(dispatch_ref(&label)))
            } else {
                None
            };
            snap.panes.push(PaneRow {
                label,
                kind,
                key: tag.clone(),
                tag,
                status: status_of(&s(p, "agent_status")).to_string(),
                name: if pname.is_empty() {
                    clean(&s(p, "label"))
                } else {
                    pname
                },
                title: clean(&s(p, "terminal_title_stripped")),
                sid,
                codex,
                // 別 session の tab は今の画面に無いので移動の対象にしない
                tab_id: Some(s(p, "tab_id")).filter(|t| !t.is_empty() && !foreign),
                ws: (name.clone(), s(p, "workspace_id")),
                socket: socket.clone(),
                pane,
            });
        }
    }
    snap.panes
        .sort_by(|a, b| a.pane.cmp(&b.pane).then(a.key.cmp(&b.key)));
    if let Some(runs) = &env.runs {
        let default_socket = canon(&env.herdr_dir.join("herdr.sock").to_string_lossy());
        read_runs(runs, &observed_sockets, &default_socket, &mut snap);
    }
    mark_dispatched(&mut snap);
    if !env.keep_hidden && !env.show_resident {
        snap.panes.retain(|r| !r.hidden());
    }
    snap
}

/// dispatch の仮の分類 (label の形が合う pane) を、run の台帳に属さないものだけ残して確定する。
/// 台帳に属すとは、run の inbox の msg が pane id と socket で帰属するか、run.md の orchestrator_pane であること。
/// since は transcript 無しでも判定したいので下限なしにする
fn mark_dispatched(snap: &mut Snapshot) {
    let is_dispatch = |r: &PaneRow| matches!(r.kind, Some(PaneKind::Dispatch(_)));
    // run が読めない (--runs-dir 不達・run 無し) ときは台帳を判定できないので付けない。
    // 候補が無ければ inbox 全体を parse しない
    if snap.runs.is_empty() || !snap.panes.iter().any(is_dispatch) {
        for r in snap.panes.iter_mut().filter(|r| is_dispatch(r)) {
            r.kind = None;
        }
        return;
    }
    // 本文は要らないので、ヘッダの (from, from_socket) だけを cache 越しに引く
    let origins: std::collections::HashSet<(String, String)> = snap
        .files
        .iter()
        .filter_map(|(_, p)| msg_origin(p))
        .map(|(f, s)| (f, canon(&s))) // cache は生の値。socket が後から実在しても合うよう毎周期 canon する
        .collect();
    let runs = &snap.runs;
    for r in snap.panes.iter_mut().filter(|r| is_dispatch(r)) {
        // since は見ず pane id と socket の一致だけで判定するので、古い msg が残っていても
        // 台帳に属すとみなして付けない (安全側)
        let in_ledger = runs.iter().any(|x| {
            x.orch == r.pane
                && x.orch_socket
                    .as_deref()
                    .is_none_or(|s| canon(s) == canon(&r.socket))
        }) || origins.contains(&(r.pane.clone(), canon(&r.socket)));
        if in_ledger {
            r.kind = None;
        }
    }
}

type OriginCache = HashMap<PathBuf, (SystemTime, u64, Option<(String, String)>)>;

/// msg ヘッダの (from, from_socket) を (path, mtime, size) で cache して返す。変わらない msg は開き直さない。
/// from_socket の無い旧形式は帰属しないので None
fn msg_origin(path: &Path) -> Option<(String, String)> {
    static CACHE: OnceLock<Mutex<OriginCache>> = OnceLock::new();
    let meta = fs::metadata(path).ok()?;
    let key = (meta.modified().ok()?, meta.len());
    let mut c = CACHE.get_or_init(Default::default).lock().ok()?;
    if let Some((t, n, v)) = c.get(path) {
        if (*t, *n) == key {
            return v.clone();
        }
    }
    // 読み取り失敗は一時的かもしれないので cache せず、次の周期で再試行する
    let v = read_msg_origin(path).ok()?;
    c.insert(path.to_path_buf(), (key.0, key.1, v.clone()));
    v
}

/// 最初の空行までのヘッダだけを読む
/// 読み取り失敗は Err、header を読めたが origin が無い (旧形式) は Ok(None)
fn read_msg_origin(path: &Path) -> std::io::Result<Option<(String, String)>> {
    let (mut from, mut sock) = (None, None);
    for l in BufReader::new(fs::File::open(path)?).lines() {
        let l = l?;
        if l.is_empty() {
            break;
        }
        match l.split_once(':') {
            Some(("from", v)) => from = Some(v.trim().to_string()),
            Some(("from_socket", v)) if !v.trim().is_empty() => sock = Some(v.trim().to_string()),
            _ => {}
        }
    }
    Ok(from.zip(sock))
}

fn mtime(p: &Path) -> Option<i64> {
    let t = fs::metadata(p).ok()?.modified().ok()?;
    Some(t.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs() as i64)
}

fn msg_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(n, _)| n.ends_with(".msg"))
        .collect();
    v.sort();
    v
}

/// ヘッダは空行まで key: value、本文はその後
pub fn parse_msg(text: &str) -> Option<Msg> {
    let (head, body) = text.split_once("\n\n").unwrap_or((text, ""));
    let h: HashMap<&str, &str> = head
        .lines()
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim(), v.trim()))
        .collect();
    Some(Msg {
        from_pane: h.get("from")?.to_string(),
        from_socket: h
            .get("from_socket")
            .filter(|v| !v.is_empty())
            .map(|v| canon(v)),
        ts: parse_time(h.get("sent_at")?)?,
        tag: h.get("tag").unwrap_or(&"").to_string(),
        body: clean(&first_line(body)),
    })
}

/// run は RUNS/<run>/run.md を持つディレクトリ。inbox は new と cur だけを見る (tmp などの書きかけを拾わない)
fn read_runs(
    dir: &Path,
    observed_sockets: &HashSet<String>,
    default_socket: &str,
    snap: &mut Snapshot,
) {
    let mut names: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    names.sort();
    for run in names {
        let Ok(md) = fs::read(run.join("run.md")) else {
            continue;
        };
        let md = String::from_utf8_lossy(&md);
        let orch = md
            .lines()
            .find_map(|l| l.strip_prefix("orchestrator_pane:"))
            .map(clean)
            .unwrap_or_default();
        let orch_socket = md
            .lines()
            .find_map(|l| l.strip_prefix("orchestrator_socket:"))
            .map(clean)
            .filter(|s| !s.is_empty());
        // default の窓口は socket を台帳に書かない場合がある。
        let socket = orch_socket.as_deref().map(canon);
        let socket = socket.as_deref().unwrap_or(default_socket);
        let window = if orch.is_empty()
            && md
                .lines()
                .any(|l| l.starts_with("closed_orchestrator_pane:"))
        {
            RunWindow::Closed
        } else if orch.is_empty() || orch == "unknown" || !observed_sockets.contains(socket) {
            RunWindow::Unknown
        } else if let Some(pane) = snap
            .panes
            .iter()
            .find(|p| p.pane == orch && p.socket == socket)
        {
            RunWindow::Active(if pane.name.is_empty() {
                pane.pane.clone()
            } else {
                pane.name.clone()
            })
        } else {
            RunWindow::Absent
        };
        let new = msg_files(&run.join("inbox/new"));
        snap.runs.push(Run {
            id: clean(&run.file_name().unwrap_or_default().to_string_lossy()),
            window,
            orch,
            orch_socket,
            new: new.len(),
            oldest: new.iter().filter_map(|(_, p)| mtime(p)).min(),
        });
        snap.files.extend(new);
        snap.files.extend(msg_files(&run.join("inbox/cur")));
    }
}

/// msg の名前は <prio>-<micros>-<tag>-<from>-<rand>.msg。from は ':' や '-' を含み得るが、tag は先頭から 3 番目で決まる
fn name_tag(file: &str) -> Option<&str> {
    file.strip_suffix(".msg")?.split('-').nth(2)
}

/// only_progress なら名前の tag が progress のものだけを開く (全 pane の進捗を毎周期見るので、他の msg は開かない)
pub fn read_msgs(files: &[(String, PathBuf)], only_progress: bool) -> Vec<InMsg> {
    files
        .iter()
        .filter(|(n, _)| !only_progress || name_tag(n) == Some("progress"))
        .filter_map(|(file, path)| {
            let b = fs::read(path).ok()?;
            let msg = parse_msg(&String::from_utf8_lossy(&b))?;
            Some(InMsg {
                file: file.clone(),
                path: path.clone(),
                msg,
            })
        })
        .collect()
}

/// pane に帰属した msg をファイル名順に。位置が seq になるので readdir の順に依らせない
pub fn mine<'a>(msgs: &'a [InMsg], p: &core::Pane) -> Vec<&'a InMsg> {
    let mut v: Vec<&InMsg> = msgs.iter().filter(|m| attributed(p, &m.msg)).collect();
    v.sort_by(|a, b| (&a.file, &a.path).cmp(&(&b.file, &b.path)));
    v
}

/// projects 直下のディレクトリ。glob を使わず read_dir して session_id のファイル名を join する
fn project_dirs(projects: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(projects)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

#[derive(Default)]
pub struct TranscriptPaths {
    projects: Vec<PathBuf>,
    rollouts: HashMap<String, PathBuf>,
}

impl TranscriptPaths {
    pub fn new(env: &Env, panes: &[PaneRow]) -> Self {
        let wanted: HashSet<&str> = panes
            .iter()
            .filter(|p| p.codex)
            .filter_map(|p| p.sid.as_deref().filter(|s| uuid(s)))
            .collect();
        Self {
            projects: project_dirs(&env.projects),
            rollouts: if wanted.is_empty() {
                HashMap::new()
            } else {
                codex_rollouts(&env.codex_sessions, &wanted).unwrap_or_default()
            },
        }
    }

    pub fn find(&self, row: &PaneRow) -> Option<PathBuf> {
        let sid = row.sid.as_deref().filter(|s| valid_sid(s))?;
        if row.codex {
            return self.rollouts.get(sid).cloned();
        }
        self.projects
            .iter()
            .map(|d| d.join(format!("{sid}.jsonl")))
            .find(|p| p.is_file())
    }
}

fn uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

fn dated_dirs(path: &Path, len: usize) -> std::io::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(path)? {
        let e = entry?;
        let name = e.file_name();
        if name
            .to_str()
            .is_some_and(|s| s.len() == len && s.bytes().all(|b| b.is_ascii_digit()))
            && e.file_type()?.is_dir()
        {
            dirs.push(e.path());
        }
    }
    Ok(dirs)
}

fn codex_rollouts(
    sessions: &Path,
    wanted: &HashSet<&str>,
) -> std::io::Result<HashMap<String, PathBuf>> {
    let mut days = vec![sessions.to_path_buf()];
    for len in [4, 2, 2] {
        let mut next = Vec::new();
        for day in days {
            next.extend(dated_dirs(&day, len)?);
        }
        days = next;
    }
    let mut candidates: HashMap<String, Option<PathBuf>> = HashMap::new();
    // 読み飛ばしたディレクトリに同じ ID の候補があるかもしれない。
    for day in days {
        for entry in fs::read_dir(day)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(id) = name
                .to_str()
                .and_then(rollout_id)
                .filter(|id| wanted.contains(id))
            else {
                continue;
            };
            if entry.file_type()?.is_file() {
                candidates
                    .entry(id.to_owned())
                    .and_modify(|p| *p = None)
                    .or_insert_with(|| Some(entry.path()));
            }
        }
    }
    Ok(candidates
        .into_iter()
        .filter_map(|(id, path)| {
            let path = path?;
            open_transcript(&path, Some(&id))?;
            Some((id, path))
        })
        .collect())
}

fn rollout_id(name: &str) -> Option<&str> {
    let name = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    name.get(name.len().checked_sub(37)?..)?
        .strip_prefix('-')
        .filter(|id| uuid(id))
}

fn open_transcript(path: &Path, codex_id: Option<&str>) -> Option<fs::File> {
    let Some(id) = codex_id else {
        return fs::File::open(path).ok();
    };
    let mut f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    if !f.metadata().ok()?.is_file() {
        return None;
    }
    // session_meta に base instructions が入るため、通常の行より広い上限で先頭 1 行だけ読む。
    let mut header = String::new();
    BufReader::new((&mut f).take(2 * 1024 * 1024))
        .read_line(&mut header)
        .ok()?;
    let meta: Value = serde_json::from_str(&header).ok()?;
    if meta["type"].as_str() != Some("session_meta") || meta["payload"]["id"].as_str() != Some(id) {
        return None;
    }
    // 同じ FD で本文を読むので、パスが入れ替わっても別セッションを表示しない。
    f.rewind().ok()?;
    Some(f)
}

/// 経過の起点に使う transcript の最終書き込み。中身は読まない
pub fn transcript_mtime(paths: &TranscriptPaths, row: &PaneRow) -> Option<i64> {
    mtime(&paths.find(row)?)
}

#[derive(Clone, Default)]
pub struct Parsed {
    pub events: Vec<Event>,
    pub first: Option<i64>,
    /// 指示 (classify が指示か貼り付け) の時刻と、含む brief のパス
    pub instrs: Vec<(i64, Option<String>)>,
}

fn to_line(v: &Value) -> Line {
    let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    let txt = |k: &str| match v.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(x) => x.to_string(),
        None => String::new(),
    };
    Line {
        ts: v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_time),
        typ: s(v, "type"),
        is_meta: b("isMeta"),
        is_sidechain: b("isSidechain"),
        is_compact_summary: b("isCompactSummary"),
        content: v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .map(String::from),
        pr: v.get("prUrl").and_then(Value::as_str).map(String::from),
        pr_label: format!("{}#{}", txt("prRepository"), txt("prNumber")),
    }
}

/// 巨大な行 (tool の出力など) を JSON にしないよう、user / pr-link の行だけを解釈する。
/// 他の行も空の Line として残すのは、連番を transcript の行番号に揃えるため
pub fn parse_lines(bytes: &[u8]) -> Vec<Line> {
    String::from_utf8_lossy(bytes)
        .split('\n')
        .map(|raw| {
            if !(raw.contains("\"type\":\"user\"") || raw.contains("\"type\":\"pr-link\"")) {
                return Line::default();
            }
            serde_json::from_str::<Value>(raw)
                .map(|v| to_line(&v))
                .unwrap_or_default()
        })
        .collect()
}

fn codex_line(raw: &str, id: &str, is_header: bool) -> Line {
    if !is_header && !raw.contains("\"UserMessage\"") {
        return Line::default();
    }
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return Line::default();
    };
    let ts = v["timestamp"].as_str().and_then(parse_time);
    if is_header && v["type"].as_str() == Some("session_meta") {
        return Line {
            ts,
            ..Line::default()
        };
    }
    let payload = &v["payload"];
    let item = &payload["item"];
    if v["type"].as_str() != Some("event_msg")
        || payload["type"].as_str() != Some("item_completed")
        || payload["thread_id"].as_str() != Some(id)
        || item["type"].as_str() != Some("UserMessage")
    {
        return Line::default();
    }
    let content = arr(item, "content")
        .iter()
        .filter(|val| val["type"].as_str() == Some("text"))
        .filter_map(|val| val["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Line {
        ts,
        typ: "user".into(),
        content: Some(content).filter(|s| !s.is_empty()),
        ..Line::default()
    }
}

fn transcript_lines(bytes: &[u8], codex_id: Option<&str>, beginning: bool) -> Vec<Line> {
    if let Some(id) = codex_id {
        String::from_utf8_lossy(bytes)
            .split('\n')
            .enumerate()
            .map(|(i, line)| codex_line(line, id, beginning && i == 0))
            .collect()
    } else {
        parse_lines(bytes)
    }
}

/// 指示 (classify が指示か貼り付け) なら (時刻, 含む brief のパス)
fn instr_brief(l: &Line) -> Option<(i64, Option<String>)> {
    if l.typ != "user" || core::excluded(l) {
        return None;
    }
    let (ts, c) = (l.ts?, l.content.as_deref()?);
    matches!(core::classify(c), Class::Instr | Class::Paste).then(|| (ts, brief_path(c)))
}

/// 新しい turn の始まり。tool result (content が配列) と compact の要約・meta・sidechain・skip 対象は数えない。
/// auto-handover の「続けて」は通常の入力として届くので数える (実 transcript で確認)。
/// skill 起動などのスラッシュコマンド (content が <command-message> で始まる) は人が始めた turn だが
/// classify は Skip にするので別に数える。モデルに送られないローカルコマンド (/compact など、<command-name> で始まる) と
/// task-notification と Stop hook の再開は起点にしない
fn is_turn(l: &Line) -> bool {
    l.typ == "user" && !core::excluded(l) && l.content.as_deref().is_some_and(is_input)
}

/// pane が受けた入力として数える内容か。turn の起点とキューの pending で同じ基準を使う
fn is_input(c: &str) -> bool {
    core::trim_left(c).starts_with("<command-message>")
        || matches!(
            core::classify(c),
            Class::Instr | Class::Paste | Class::HerdrMsg
        )
}

/// キューに残っている件数の上限。session が落ちて dequeue / remove が書かれなかった行が溜まり続けないよう、古い方から捨てる
const QUEUE_MAX: usize = 256;
/// 件数に数えない分も含めて、FIFO と content の突き合わせ用に覚えておく上限
const QUEUE_KEEP: usize = QUEUE_MAX * 4;

/// transcript の queue-operation を畳んだ、pane の入力キュー。Lean の model の外 (core.rs には置かない)。
/// 実 transcript で確かめた形:
/// - enqueue は content を持つ。dequeue は content を持たず先頭から外す。FIFO と content の無い remove、
///   dequeue の書かれない user 行・queued_command への到達を下の規則で畳むと、561 session (jsonl) の replay で、
///   後に turn が続くのに残る入力らしい行は 1 件 (入力に数えない task-notification) になった。
///   残りは session の終わりに未到達だった行 (EOF)
/// - remove は enqueue と同じ content と reason を持つ (content の無いものもあり、そのときは最古の行を外す)。
///   reason が dropped_by_hook 以外は届いた (mid-turn に取り込まれた)
/// - dequeue が書かれないまま同じ content の user 行・queued_command の attachment で届く入力は、その行を外す
///   (届いた時刻は user 行側が持つので delivered_ts には触れない)
/// - popAll は入力欄へ戻されたのでキューが空になる
/// - queued_command の attachment の時刻は送った時刻 (enqueue と同じ) で、届いた時刻は remove の時刻。
///   attachment は remove と同じ content を持つので、キューの出入りは queue-operation だけから取る
#[derive(Clone, Default)]
pub struct Queue {
    /// (enqueue の時刻, 入力として数えるか, content の hash)。content は大きいので持たない
    items: Vec<(i64, bool, u64)>,
    /// 入力がキューを出て pane に届いた最後の時刻 (dequeue と、remove の reason が dropped_by_hook 以外)
    delivered_ts: Option<i64>,
    /// dequeue / remove で出たが、まだ user 行・queued_command が来ていない入力の hash。
    /// 同じ content の別の入力を、その到達で二重に外さないために持つ
    claimed: Vec<u64>,
    /// items の先頭のうち、上限 QUEUE_MAX を超えて pending に数えない件数。FIFO と content の突き合わせには残す
    skip: usize,
}

fn content_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.trim().hash(&mut h);
    h.finish()
}

impl Queue {
    fn apply(&mut self, ts: i64, v: &Value) {
        let content = v.get("content").and_then(Value::as_str).unwrap_or("");
        match v.get("operation").and_then(Value::as_str) {
            Some("enqueue") => {
                self.items
                    .push((ts, is_input(content), content_hash(content)));
                if self.items.len() - self.skip > QUEUE_MAX {
                    self.skip += 1;
                }
                // dequeue / remove が書かれない session で items が増え続けないよう、突き合わせ用も打ち切る
                if self.items.len() > QUEUE_KEEP {
                    self.take(0);
                }
            }
            Some("dequeue") if !self.items.is_empty() => {
                let it = self.take(0);
                self.claim(it);
                self.deliver(ts, it.1);
            }
            Some("remove") => {
                let h = content_hash(content);
                let pos = if content.is_empty() {
                    (!self.items.is_empty()).then_some(0)
                } else {
                    self.items.iter().position(|x| x.2 == h)
                };
                if let Some(i) = pos {
                    let it = self.take(i);
                    if v.get("reason").and_then(Value::as_str) != Some("dropped_by_hook") {
                        self.claim(it);
                        self.deliver(ts, it.1);
                    }
                }
            }
            Some("popAll") => {
                self.items.clear();
                self.claimed.clear();
                self.skip = 0;
            }
            _ => {}
        }
    }

    /// dequeue が書かれないまま user 行・queued_command で届いた入力。その content の最初の一致を外す
    fn arrived(&mut self, content: &str) {
        let h = content_hash(content);
        // dequeue / remove が既に外した入力の到達なら、同じ content の別の入力は残す
        if let Some(i) = self.claimed.iter().position(|&c| c == h) {
            self.claimed.remove(i);
        } else if let Some(i) = self.items.iter().position(|x| x.2 == h) {
            self.take(i);
        }
    }

    fn take(&mut self, i: usize) -> (i64, bool, u64) {
        self.skip = self.skip.saturating_sub(usize::from(i < self.skip));
        self.items.remove(i)
    }

    fn deliver(&mut self, ts: i64, inp: bool) {
        if inp {
            self.delivered_ts = Some(self.delivered_ts.map_or(ts, |p| p.max(ts)));
        }
    }

    /// 入力を dequeue / remove で外したとき、後に来るその到達を数え済みにする
    fn claim(&mut self, it: (i64, bool, u64)) {
        if it.1 {
            self.claimed.push(it.2);
            if self.claimed.len() > QUEUE_MAX {
                self.claimed.remove(0);
            }
        }
    }

    /// 届いていない入力の件数と、その最古の enqueue の時刻
    pub fn pending(&self) -> (usize, Option<i64>) {
        let ts = || self.items[self.skip..].iter().filter(|x| x.1).map(|x| x.0);
        (ts().count(), ts().min())
    }
}

/// 解析結果のキャッシュ。開いている Timeline は 1 つなので 1 件だけ持つ
#[derive(Default)]
pub struct Transcripts {
    key: Option<(PathBuf, SystemTime, u64)>,
    val: Parsed,
    pub parses: usize,
}

impl Transcripts {
    /// 読めない transcript はイベント 0 件。失敗はキャッシュしない (権限を直したら次の周期で読む)
    pub fn get(&mut self, path: &Path, codex: bool) -> Parsed {
        let key = fs::metadata(path)
            .ok()
            .and_then(|m| Some((path.to_path_buf(), m.modified().ok()?, m.len())));
        if key.is_some() && key == self.key {
            return self.val.clone();
        }
        let lines = (|| {
            let codex_id = if codex {
                Some(rollout_id(path.file_name()?.to_str()?)?)
            } else {
                None
            };
            let mut bytes = Vec::new();
            open_transcript(path, codex_id)?
                .read_to_end(&mut bytes)
                .ok()?;
            Some(transcript_lines(&bytes, codex_id, true))
        })();
        let events = transcript_events(lines.as_deref());
        let Some(lines) = lines else {
            return Parsed::default();
        };
        self.parses += 1;
        let first = lines.iter().filter_map(|l| l.ts).min();
        let instrs = lines.iter().filter_map(instr_brief).collect();
        self.val = Parsed {
            events,
            first,
            instrs,
        };
        self.key = key;
        self.val.clone()
    }
}

/// pane 行の要約と step の行の元。brief を読めなければ goal は None
#[derive(Default)]
pub struct Goal {
    pub goal: Option<String>,
    pub steps: Vec<String>,
    pub status: Option<Status>,
}

/// transcript ごとの追跡状態。transcript は追記だけなので、前回読んだ位置 (offset) から後ろだけを読む
#[derive(Clone, Default)]
pub struct BriefTracker {
    offset: u64,
    pub first_ts: Option<i64>,
    pub last_brief: Option<String>,
    pub last_brief_ts: Option<i64>,
    /// 最後に pane が受けた入力 (人の指示・貼り付け・他 pane からの message) の時刻。working の経過の起点
    pub last_turn_ts: Option<i64>,
    pub queue: Queue,
}

impl BriefTracker {
    /// 最後に入力が pane へ届いた時刻。turn の起点に、キューから出て mid-turn に取り込まれた入力を足したもの
    pub fn last_input_ts(&self) -> Option<i64> {
        self.last_turn_ts.max(self.queue.delivered_ts)
    }
}

/// 全 pane の行にゴールと進捗を出すための軽い追跡。Timeline 用の 1 件キャッシュ (Transcripts) とは別に持つ
#[derive(Default)]
pub struct Briefs {
    tracks: HashMap<PathBuf, BriefTracker>,
    docs: HashMap<String, (SystemTime, Option<String>, Vec<String>)>,
    /// transcript から読んだバイト数。差分だけを読んでいることをテストで確かめる
    pub read_bytes: u64,
}

impl Briefs {
    /// 読めなければ状態を変えず None (ゴール無しで続ける)
    pub fn scan(&mut self, path: &Path, codex: bool) -> Option<BriefTracker> {
        let codex_id = if codex {
            Some(rollout_id(path.file_name()?.to_str()?)?)
        } else {
            None
        };
        let mut f = open_transcript(path, codex_id)?;
        let size = f.metadata().ok()?.len();
        let mut t = self.tracks.get(path).cloned().unwrap_or_default();
        if size < t.offset {
            t = BriefTracker::default(); // 縮んだら別物とみなして最初から読む
        }
        f.seek(SeekFrom::Start(t.offset)).ok()?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok()?;
        self.read_bytes += buf.len() as u64;
        // 書きかけの行は次の周期に回す
        let end = buf.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        let text = String::from_utf8_lossy(&buf[..end]);
        // parse_lines は split('\n') と同じ行数・同じ並びで返すので zip できる。queue は順序どおりに畳む
        for (raw, l) in text
            .split('\n')
            .zip(transcript_lines(&buf[..end], codex_id, t.offset == 0))
        {
            if !codex && raw.contains("\"type\":\"queue-operation\"") {
                if let Ok(v) = serde_json::from_str::<Value>(raw) {
                    let ts = v
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .and_then(parse_time);
                    if let (Some(ts), Some("queue-operation")) =
                        (ts, v.get("type").and_then(Value::as_str))
                    {
                        t.queue.apply(ts, &v);
                    }
                }
            } else if !codex && raw.contains("\"queued_command\"") {
                if let Ok(v) = serde_json::from_str::<Value>(raw) {
                    let a = v.get("attachment");
                    if a.and_then(|a| a.get("type")).and_then(Value::as_str)
                        == Some("queued_command")
                    {
                        if let Some(p) = a.and_then(|a| a.get("prompt")).and_then(Value::as_str) {
                            t.queue.arrived(p);
                        }
                    }
                }
            }
            if l.typ == "user" && !core::excluded(&l) {
                if let Some(c) = l.content.as_deref() {
                    t.queue.arrived(c);
                }
            }
            let Some(ts) = l.ts else { continue };
            t.first_ts = Some(t.first_ts.map_or(ts, |f| f.min(ts)));
            if is_turn(&l) {
                t.last_turn_ts = Some(t.last_turn_ts.map_or(ts, |p| p.max(ts)));
            }
            if let Some((ts, b @ Some(_))) = instr_brief(&l) {
                t.last_brief = last_brief(&[t.last_brief.take(), b]);
                t.last_brief_ts = Some(ts);
            }
        }
        t.offset += end as u64;
        self.tracks.insert(path.to_path_buf(), t.clone());
        Some(t)
    }

    /// brief の H1 と steps。(path, mtime) でキャッシュする
    fn doc(&mut self, path: &str) -> Option<(Option<String>, Vec<String>)> {
        let m = fs::metadata(path).ok()?.modified().ok()?;
        if let Some((_, g, s)) = self.docs.get(path).filter(|d| d.0 == m) {
            return Some((g.clone(), s.clone()));
        }
        let text = fs::read(path).ok()?;
        let text = String::from_utf8_lossy(&text);
        let (g, s) = (goal_of(&text), parse_steps(&text));
        self.docs
            .insert(path.to_string(), (m, g.clone(), s.clone()));
        Some((g, s))
    }

    /// since は brief を指示した時刻。それより前の progress は前の brief に対するものなので数えない
    fn goal(&mut self, brief: &str, since: i64, mine: &[&InMsg]) -> Goal {
        let Some((goal, steps)) = self.doc(brief) else {
            return Goal::default();
        };
        let ps: Vec<Progress> = mine
            .iter()
            .enumerate()
            .filter(|(_, m)| m.msg.tag == "progress" && m.msg.ts >= since)
            .map(|(i, m)| Progress {
                ts: m.msg.ts,
                seq: i,
                names: progress_names(&m.msg.body),
            })
            .collect();
        Goal {
            goal,
            status: Some(status(&steps, &ps)),
            steps,
        }
    }

    /// working の経過の起点。transcript の最後の turn の時刻。読めない、または turn が無いときは None
    pub fn turn_start(&mut self, row: &PaneRow, paths: &TranscriptPaths) -> Option<i64> {
        let p = paths.find(row)?;
        self.scan(&p, row.codex)?.last_turn_ts
    }

    /// pane 行のゴール。progress は read_msgs(.., true) で読んだ msg
    pub fn pane_goal(
        &mut self,
        row: &PaneRow,
        paths: &TranscriptPaths,
        progress: &[InMsg],
    ) -> Goal {
        let Some(t) = paths.find(row).and_then(|p| self.scan(&p, row.codex)) else {
            return Goal::default();
        };
        let (Some(brief), Some(since)) = (t.last_brief, t.last_brief_ts) else {
            return Goal::default();
        };
        let p = core::Pane {
            pane: row.pane.clone(),
            socket: row.socket.clone(),
            since: t.first_ts,
        };
        self.goal(&brief, since, &mine(progress, &p))
    }
}

/// 選択した pane の Timeline・ゴール・進捗
#[derive(Default)]
pub struct Detail {
    pub has_transcript: bool,
    pub events: Vec<Event>,
    pub goal: Goal,
}

pub fn detail(
    snap: &Snapshot,
    row: &PaneRow,
    cache: &mut Transcripts,
    briefs: &mut Briefs,
    paths: &TranscriptPaths,
) -> Detail {
    let Some(path) = paths.find(row) else {
        return Detail::default();
    };
    let parsed = cache.get(&path, row.codex);
    let p = core::Pane {
        pane: row.pane.clone(),
        socket: row.socket.clone(),
        since: parsed.first,
    };
    let msgs = read_msgs(&snap.files, false);
    let mine = mine(&msgs, &p);
    let mut events = parsed.events.clone();
    events.extend(mine.iter().enumerate().map(|(i, m)| Event {
        ts: m.msg.ts,
        src: 1,
        seq: i,
        kind: Kind::Report,
        body: m.msg.body.clone(),
    }));
    let mut d = Detail {
        has_transcript: true,
        events: order(events),
        ..Default::default()
    };
    let bs: Vec<Option<String>> = parsed.instrs.iter().map(|(_, b)| b.clone()).collect();
    if let Some(brief) = last_brief(&bs) {
        let since = parsed
            .instrs
            .iter()
            .rev()
            .find(|(_, b)| b.as_ref() == Some(&brief))
            .map_or(0, |x| x.0);
        d.goal = briefs.goal(&brief, since, &mine);
    }
    d
}

/// pane 行の要約。ゴールが無ければ terminal の title
pub fn summary(g: &Goal, title: &str) -> String {
    let Some(goal) = &g.goal else {
        return squash(title);
    };
    match (&g.status, g.steps.len()) {
        (_, 0) | (None, _) => goal.clone(),
        (Some(Status::Unreported), _) => format!("{goal}  進捗未報告"),
        (Some(Status::Reported { done, .. }), m) => {
            match g.steps.iter().find(|s| !done.contains(s)) {
                Some(next) => format!("{goal}  {}/{m} {next}", done.len()),
                None => format!("{goal}  {m}/{m}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("herdr-timeline-unit-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    const T: &str =
        r#"{"type":"user","timestamp":"2026-01-02T03:04:05Z","message":{"content":"直して"}}"#;

    #[test]
    fn dispatched_label_shape() {
        for l in ["repo-123", "repo-bundle-7", "a-b-1"] {
            assert!(is_dispatched_label(l), "{l} は dispatch の形のはず");
        }
        for l in ["watch-worker", "feat-xxx", "-12", "repo-12a", "repo-", ""] {
            assert!(!is_dispatched_label(l), "{l:?} は dispatch の形でないはず");
        }
    }

    #[test]
    fn resident_labels_match_exactly() {
        // 実装の定数とは別に期待値を持つ (定数と同じ一覧を回すだけだと変更に気づけない)
        let expected = [
            "watch-my-pr-reviews",
            "watch-other-pr-reviews",
            "watch-worker",
            "review-submit",
            "Cue回答の監視",
        ];
        for l in expected {
            assert!(is_resident(l), "{l} は常駐のはず");
        }
        assert_eq!(
            RESIDENT_LABELS.len(),
            expected.len(),
            "RESIDENT_LABELS と試験の一覧の数が違う。一覧を変えたら試験も更新する"
        );
        for l in [
            "impl-task",
            "watch-worker2",
            "Watch-worker",
            "watch-worker ",
            "",
        ] {
            assert!(!is_resident(l), "{l:?} は常駐でないはず (完全一致のみ)");
        }
    }

    #[test]
    fn req5_cache_by_path_mtime_size() {
        let d = tmp("req5");
        let f = d.join("s.jsonl");
        fs::write(&f, format!("{T}\n")).unwrap();
        let mut c = Transcripts::default();
        assert_eq!(c.get(&f, false).events.len(), 1, "1 件読めるはず");
        assert_eq!(c.get(&f, false).events.len(), 1, "2 回目も同じ結果のはず");
        assert_eq!(c.parses, 1, "同じ mtime と size なら解析し直さないはず");
        fs::write(&f, format!("{T}\n{T}\n")).unwrap();
        assert_eq!(c.get(&f, false).events.len(), 2, "書き換えた内容を読むはず");
        assert_eq!(c.parses, 2, "書き換えたら解析し直すはず");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn msg_origin_cache_follows_file_change() {
        let d = tmp("origin");
        let f = d.join("a.msg");
        fs::write(&f, "from: w:p1\nfrom_socket: /s/a\n\n本文\n").unwrap();
        let a = msg_origin(&f);
        assert_eq!(
            a.as_ref().map(|x| x.0.as_str()),
            Some("w:p1"),
            "from を読むはず"
        );
        assert_eq!(msg_origin(&f), a, "変わらなければ同じ値のはず");
        fs::write(&f, "from: w:p2\nfrom_socket: /s/a\n\n本文 長くした\n").unwrap();
        assert_eq!(
            msg_origin(&f).map(|x| x.0),
            Some("w:p2".to_string()),
            "size が変われば新しい値のはず"
        );
        fs::write(&f, "from: w:p2\n\n本文\n").unwrap();
        assert_eq!(msg_origin(&f), None, "旧形式は帰属しないはず");
        let _ = fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn msg_origin_read_failure_is_err_and_not_cached() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp("origin-err");
        let f = d.join("a.msg");
        fs::write(&f, "from: w:p1\nfrom_socket: /s/a\n\n本文\n").unwrap();
        let mode = |m| fs::set_permissions(&f, fs::Permissions::from_mode(m)).unwrap();
        mode(0o000);
        // root は 000 でも読めてしまい失敗を作れないので skip する
        if fs::File::open(&f).is_ok() {
            mode(0o644);
            let _ = fs::remove_dir_all(&d);
            return;
        }
        // 失敗時と回復時で (path, mtime, size) の key は同じ。失敗を cache すると回復後も None のままになる
        assert!(read_msg_origin(&f).is_err(), "読めない path は Err のはず");
        assert_eq!(msg_origin(&f), None, "読めない間は None のはず");
        mode(0o644);
        assert_eq!(
            msg_origin(&f).map(|x| x.0),
            Some("w:p1".to_string()),
            "直したら値が返る (失敗を cache しない) はず"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn msg_origin_keeps_raw_socket() {
        let d = tmp("origin-raw");
        // canon が入力を変える実在 path を使う (存在しない path だと canon は恒等で区別できない)
        fs::create_dir(d.join("sub")).unwrap();
        fs::write(d.join("sock"), "").unwrap();
        let raw = format!("{}/sub/../sock", d.display());
        let f = d.join("a.msg");
        fs::write(&f, format!("from: w:p1\nfrom_socket:  {raw} \n\n")).unwrap();
        let got = msg_origin(&f).map(|x| x.1);
        assert_eq!(got, Some(raw.clone()), "canon 前の trim 済みの値のはず");
        assert_ne!(got, Some(canon(&raw)), "canon 後の値ではないはず");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn req1_unreadable_or_broken_transcript_is_empty() {
        let d = tmp("req1");
        let mut c = Transcripts::default();
        assert!(
            c.get(&d.join("missing.jsonl"), false).events.is_empty(),
            "存在しない transcript は 0 件のはず"
        );
        let f = d.join("locked.jsonl");
        fs::write(&f, format!("{T}\n")).unwrap();
        // SAFETY: geteuid は引数を取らず失敗しない
        if unsafe { libc::geteuid() } != 0 {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&f, fs::Permissions::from_mode(0o000)).unwrap();
            assert!(
                c.get(&f, false).events.is_empty(),
                "権限の無い transcript は 0 件のはず"
            );
            fs::set_permissions(&f, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let mut bad = b"\xff\xfe broken\n".to_vec();
        bad.extend_from_slice(T.as_bytes());
        assert_eq!(
            transcript_events(Some(&parse_lines(&bad))).len(),
            1,
            "途中の行が UTF-8 でなくても他の行は読むはず"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn turn_start_follows_last_input_only() {
        let d = tmp("turn");
        let f = d.join("s.jsonl");
        let user = |at: &str, content: &str, extra: &str| {
            format!(
                r#"{{"type":"user","timestamp":"{at}",{extra}"message":{{"content":{content}}}}}"#
            ) + "\n"
        };
        let mut s = user("2026-01-02T03:00:00Z", r#""直して""#, "");
        // tool result (配列)、compact の要約、meta は turn でない
        s += &user("2026-01-02T03:10:00Z", r#"[{"type":"tool_result"}]"#, "");
        s += &user(
            "2026-01-02T03:11:00Z",
            r#""要約""#,
            r#""isCompactSummary":true,"#,
        );
        s += &user("2026-01-02T03:12:00Z", r#""meta""#, r#""isMeta":true,"#);
        fs::write(&f, &s).unwrap();
        let mut b = Briefs::default();
        let t0 = parse_time("2026-01-02T03:00:00Z");
        assert_eq!(
            b.scan(&f, false).unwrap().last_turn_ts,
            t0,
            "人の指示だけが turn のはず"
        );
        // auto-handover の「続けて」は通常の入力なので新しい turn
        let l = user("2026-01-02T03:20:00Z", r#""続けて""#, "");
        fs::OpenOptions::new()
            .append(true)
            .open(&f)
            .unwrap()
            .write_all(l.as_bytes())
            .unwrap();
        assert_eq!(
            b.scan(&f, false).unwrap().last_turn_ts,
            parse_time("2026-01-02T03:20:00Z"),
            "「続けて」は新しい turn のはず"
        );
        // スラッシュコマンドも新しい turn。task-notification は起点にしない
        let t1 = parse_time("2026-01-02T03:30:00Z");
        let l = user(
            "2026-01-02T03:30:00Z",
            r#""<command-message>front-desk</command-message>\n<command-name>/front-desk</command-name>""#,
            "",
        );
        fs::OpenOptions::new()
            .append(true)
            .open(&f)
            .unwrap()
            .write_all(l.as_bytes())
            .unwrap();
        assert_eq!(
            b.scan(&f, false).unwrap().last_turn_ts,
            t1,
            "スラッシュコマンドは新しい turn のはず"
        );
        let l = user(
            "2026-01-02T03:40:00Z",
            r#""<task-notification>x</task-notification>""#,
            "",
        );
        fs::OpenOptions::new()
            .append(true)
            .open(&f)
            .unwrap()
            .write_all(l.as_bytes())
            .unwrap();
        assert_eq!(
            b.scan(&f, false).unwrap().last_turn_ts,
            t1,
            "task-notification は turn の起点にならないはず"
        );
        // モデルに送られないローカルコマンドは起点にしない
        let l = user(
            "2026-01-02T03:50:00Z",
            r#""<command-name>/compact</command-name>\n            <command-message>compact</command-message>""#,
            "",
        );
        fs::OpenOptions::new()
            .append(true)
            .open(&f)
            .unwrap()
            .write_all(l.as_bytes())
            .unwrap();
        assert_eq!(
            b.scan(&f, false).unwrap().last_turn_ts,
            t1,
            "ローカルコマンドは turn の起点にならないはず"
        );
        let _ = fs::remove_dir_all(&d);
    }

    fn qline(op: &str, at: &str, content: &str, reason: &str) -> String {
        let mut v =
            serde_json::json!({"type": "queue-operation", "operation": op, "timestamp": at});
        if !content.is_empty() {
            v["content"] = content.into();
        }
        if !reason.is_empty() {
            v["reason"] = reason.into();
        }
        v.to_string() + "\n"
    }

    /// 架空の queue-operation の列。実 transcript で確かめた形 (dequeue は content 無し、remove は reason 付き) に揃える
    fn queue_script() -> Vec<String> {
        let msg = "<pasted_content id=\"a1\">\n[herdr-message] from 窓口 (wA:p1)\n\n続きをお願いします\n</pasted_content>";
        let note = "<task-notification>\n<task-id>x</task-id>\n</task-notification>";
        vec![
            // 入力が届いて処理中
            qline("enqueue", "2026-01-02T03:00:00Z", "実装して", ""),
            qline("dequeue", "2026-01-02T03:00:00Z", "", ""),
            // 処理中に他 pane の message が来てキューに残る (未到達)
            qline("enqueue", "2026-01-02T03:01:00Z", msg, ""),
            // 3 分後に mid-turn で取り込まれる (届いた)
            qline("remove", "2026-01-02T03:04:00Z", msg, "absorbed_mid_turn"),
            // task-notification は入力に数えない。dequeue は FIFO なので先に出る
            qline("enqueue", "2026-01-02T03:05:00Z", note, ""),
            qline("enqueue", "2026-01-02T03:06:00Z", "続けて", ""),
            qline("dequeue", "2026-01-02T03:07:00Z", "", ""),
            qline("dequeue", "2026-01-02T03:09:00Z", "", ""),
            // hook に捨てられた入力は届いていない。popAll は入力欄に戻されたのでキューが空になる
            qline("enqueue", "2026-01-02T03:10:00Z", "捨てられる", ""),
            qline(
                "remove",
                "2026-01-02T03:11:00Z",
                "捨てられる",
                "dropped_by_hook",
            ),
            qline("enqueue", "2026-01-02T03:12:00Z", "戻される", ""),
            qline("popAll", "2026-01-02T03:13:00Z", "戻される", ""),
        ]
    }

    #[test]
    fn queue_separates_pending_from_delivered() {
        let d = tmp("queue");
        let f = d.join("s.jsonl");
        let script = queue_script();
        let t = |s: &str| parse_time(s);
        // (取り込んだ行数, pending, 最古の enqueue, 最後に届いた時刻)
        let expect = [
            (2, 0, None, t("2026-01-02T03:00:00Z")),
            (3, 1, t("2026-01-02T03:01:00Z"), t("2026-01-02T03:00:00Z")),
            (4, 0, None, t("2026-01-02T03:04:00Z")),
            (6, 1, t("2026-01-02T03:06:00Z"), t("2026-01-02T03:04:00Z")),
            (7, 1, t("2026-01-02T03:06:00Z"), t("2026-01-02T03:04:00Z")),
            (8, 0, None, t("2026-01-02T03:09:00Z")),
            (10, 0, None, t("2026-01-02T03:09:00Z")),
            (12, 0, None, t("2026-01-02T03:09:00Z")),
        ];
        let mut b = Briefs::default();
        let mut done = 0;
        for (n, pending, oldest, last) in expect {
            let chunk: String = script[done..n].concat();
            done = n;
            let mut o = fs::OpenOptions::new();
            o.create(true).append(true);
            o.open(&f).unwrap().write_all(chunk.as_bytes()).unwrap();
            let tr = b.scan(&f, false).unwrap();
            assert_eq!(tr.queue.pending(), (pending, oldest), "{n} 行目まで");
            assert_eq!(tr.last_input_ts(), last, "{n} 行目まで");
        }
        // 1 回で全部読んでも、途中まで読んでから続きを読んでも同じ状態になる
        let once = Briefs::default().scan(&f, false).unwrap();
        let last = b.scan(&f, false).unwrap();
        assert_eq!(once.queue.pending(), last.queue.pending());
        assert_eq!(once.last_input_ts(), last.last_input_ts());
        let _ = fs::remove_dir_all(&d);
    }

    fn uline(at: &str, content: &str) -> String {
        serde_json::json!({"type": "user", "timestamp": at, "message": {"content": content}})
            .to_string()
            + "\n"
    }

    fn aline(at: &str, prompt: &str) -> String {
        serde_json::json!({"type": "attachment", "timestamp": at,
            "attachment": {"type": "queued_command", "prompt": prompt}})
        .to_string()
            + "\n"
    }

    fn pending_of(f: &Path) -> (usize, Option<i64>) {
        Briefs::default().scan(f, false).unwrap().queue.pending()
    }

    #[test]
    fn queue_same_content_is_not_consumed_twice() {
        let d = tmp("queue-same-content");
        let f = d.join("s.jsonl");
        let e = |at| qline("enqueue", at, "続けて", "");
        // dequeue が 1 件外し、その到達の user 行で 2 件目まで外してはいけない
        let s = [
            e("2026-01-02T03:00:00Z"),
            e("2026-01-02T03:00:01Z"),
            qline("dequeue", "2026-01-02T03:01:00Z", "", ""),
            uline("2026-01-02T03:01:00Z", "続けて"),
        ]
        .concat();
        fs::write(&f, &s).unwrap();
        assert_eq!(pending_of(&f).0, 1, "dequeue と user 行で 2 件消えた");
        // remove と同じ content の queued_command も同様
        let s = [
            e("2026-01-02T03:00:00Z"),
            e("2026-01-02T03:00:01Z"),
            qline(
                "remove",
                "2026-01-02T03:01:00Z",
                "続けて",
                "absorbed_mid_turn",
            ),
            aline("2026-01-02T03:00:00Z", "続けて"),
        ]
        .concat();
        fs::write(&f, &s).unwrap();
        assert_eq!(pending_of(&f).0, 1, "remove と attachment で 2 件消えた");
    }

    #[test]
    fn queue_keeps_bounded_memory_without_dequeue() {
        let mut q = Queue::default();
        for i in 0..QUEUE_KEEP * 3 {
            let v = serde_json::json!({"operation": "enqueue", "content": format!("入力{i}")});
            q.apply(0, &v);
        }
        assert!(q.items.len() <= QUEUE_KEEP, "items が上限なく増えた");
        assert_eq!(q.pending().0, QUEUE_MAX);
    }

    #[test]
    fn queue_cap_eviction_still_matches_by_content() {
        let d = tmp("queue-cap-content");
        let f = d.join("s.jsonl");
        let mut s = String::new();
        for i in 0..=QUEUE_MAX {
            let c = format!("入力{i}");
            s += &qline("enqueue", "2026-01-02T03:00:00Z", &c, "");
        }
        // 上限で数えなくなった先頭を content で外し、残りを dequeue すると 0 件になる
        s += &qline(
            "remove",
            "2026-01-02T04:00:00Z",
            "入力0",
            "absorbed_mid_turn",
        );
        for _ in 0..QUEUE_MAX {
            s += &qline("dequeue", "2026-01-02T05:00:00Z", "", "");
        }
        fs::write(&f, &s).unwrap();
        assert_eq!(
            pending_of(&f).0,
            0,
            "上限で捨てた入力の remove で FIFO がずれた"
        );
        // dequeue の無い到達 (user 行) でも同じ
        let s = s
            .lines()
            .take(QUEUE_MAX + 1)
            .map(|l| l.to_string() + "\n")
            .collect::<String>()
            + &uline("2026-01-02T04:00:00Z", "入力0");
        fs::write(&f, &s).unwrap();
        assert_eq!(pending_of(&f).0, QUEUE_MAX, "到達した先頭が pending のまま");
    }

    #[test]
    fn queue_cap_eviction_keeps_fifo_alignment() {
        let d = tmp("queue-cap-fifo");
        let f = d.join("s.jsonl");
        let note = "<task-notification>done</task-notification>";
        let mut s = String::new();
        for i in 0..QUEUE_MAX {
            s += &qline(
                "enqueue",
                &format!("2026-01-02T03:{:02}:{:02}Z", i / 60, i % 60),
                note,
                "",
            );
        }
        s += &qline("enqueue", "2026-01-02T04:00:00Z", "実装して", "");
        // 実キューは 257 件。通知 256 回の dequeue では指示を外さない
        for _ in 0..QUEUE_MAX {
            s += &qline("dequeue", "2026-01-02T05:00:00Z", "", "");
        }
        fs::write(&f, &s).unwrap();
        assert_eq!(pending_of(&f).0, 1, "上限で捨てた後に指示が先に外れた");
    }

    #[test]
    fn queue_contentless_remove_drops_oldest() {
        let d = tmp("queue-contentless");
        let f = d.join("s.jsonl");
        // content の無い remove が最古の A を外さないと、dequeue が A を外して B が残る
        let s = [
            qline("enqueue", "2026-01-02T03:00:00Z", "A", ""),
            qline("enqueue", "2026-01-02T03:01:00Z", "実装して", ""),
            qline("remove", "2026-01-02T03:02:00Z", "", "dropped_by_hook"),
            qline("dequeue", "2026-01-02T03:03:00Z", "", ""),
        ]
        .concat();
        fs::write(&f, s).unwrap();
        assert_eq!(
            pending_of(&f),
            (0, None),
            "content の無い remove で dequeue がずれたはず"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn queue_arrival_without_dequeue_by_user_line_or_attachment() {
        let d = tmp("queue-arrived");
        let f = d.join("s.jsonl");
        let at = "2026-01-02T03:05:00Z";
        let enq = qline("enqueue", "2026-01-02T03:00:00Z", "続けて", "");
        fs::write(&f, &enq).unwrap();
        assert_eq!(pending_of(&f).0, 1, "届く前は pending のはず");
        // user 行 (前後の空白は trim して比較する)
        fs::write(&f, enq.clone() + &uline(at, " 続けて\n")).unwrap();
        assert_eq!(
            pending_of(&f),
            (0, None),
            "user 行で届いたのに pending のまま"
        );
        // meta の user 行は届いた扱いにしない
        let meta = serde_json::json!({"type":"user","timestamp":at,"isMeta":true,"message":{"content":"続けて"}});
        fs::write(&f, enq.clone() + &(meta.to_string() + "\n")).unwrap();
        assert_eq!(pending_of(&f).0, 1, "meta の user 行は除くはず");
        // queued_command の attachment
        fs::write(&f, enq.clone() + &aline(at, "続けて")).unwrap();
        assert_eq!(
            pending_of(&f),
            (0, None),
            "attachment で届いたのに pending のまま"
        );
        // 届いた時刻には触れない
        let tr = Briefs::default().scan(&f, false).unwrap();
        assert_eq!(
            tr.queue.delivered_ts, None,
            "delivered_ts は user 行側が持つはず"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn queue_arrival_split_scan_matches_single_scan() {
        let d = tmp("queue-arrived-split");
        let f = d.join("s.jsonl");
        let script = [
            qline("enqueue", "2026-01-02T03:00:00Z", "一つ目", ""),
            qline("enqueue", "2026-01-02T03:01:00Z", "二つ目", ""),
            uline("2026-01-02T03:02:00Z", "一つ目"),
            qline("enqueue", "2026-01-02T03:03:00Z", "三つ目", ""),
            aline("2026-01-02T03:04:00Z", "二つ目"),
        ];
        let mut b = Briefs::default();
        let mut done = 0;
        for n in [1, 3, 4, 5] {
            let mut o = fs::OpenOptions::new();
            o.create(true).append(true);
            o.open(&f)
                .unwrap()
                .write_all(script[done..n].concat().as_bytes())
                .unwrap();
            done = n;
            b.scan(&f, false).unwrap();
        }
        let split = b.scan(&f, false).unwrap();
        let once = Briefs::default().scan(&f, false).unwrap();
        assert_eq!(
            split.queue.pending(),
            once.queue.pending(),
            "分割 scan と 1 回 scan が違う"
        );
        assert_eq!(
            once.queue.pending(),
            (1, parse_time("2026-01-02T03:03:00Z"))
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn queue_ignores_unpaired_remove_and_resets_on_shrink() {
        let d = tmp("queue-shrink");
        let f = d.join("s.jsonl");
        // enqueue の無い remove と、先頭が空のキューへの dequeue は無視する
        let mut s = qline(
            "remove",
            "2026-01-02T03:00:00Z",
            "知らない",
            "absorbed_mid_turn",
        );
        s += &qline("dequeue", "2026-01-02T03:00:01Z", "", "");
        s += &qline("enqueue", "2026-01-02T03:01:00Z", "直して", "");
        fs::write(&f, &s).unwrap();
        let mut b = Briefs::default();
        let tr = b.scan(&f, false).unwrap();
        assert_eq!(tr.queue.pending(), (1, parse_time("2026-01-02T03:01:00Z")));
        assert_eq!(tr.last_input_ts(), None);
        // 縮んだら別物として最初から読む
        fs::write(&f, qline("enqueue", "2026-01-02T04:00:00Z", "別の指示", "")).unwrap();
        let tr = b.scan(&f, false).unwrap();
        assert_eq!(tr.queue.pending(), (1, parse_time("2026-01-02T04:00:00Z")));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn brief_tracker_reads_only_appended_lines() {
        let d = tmp("tracker");
        let f = d.join("s.jsonl");
        let brief = |name: &str, at: &str| {
            format!(
                r#"{{"type":"user","timestamp":"{at}","message":{{"content":"{}/brief-{name}.md を読んで"}}}}"#,
                d.display()
            ) + "\n"
        };
        let (l1, l2) = (
            brief("a", "2026-01-02T03:00:00Z"),
            brief("b", "2026-01-02T03:20:00Z"),
        );
        fs::write(&f, &l1).unwrap();
        let mut b = Briefs::default();
        let t = b.scan(&f, false).expect("読めるはず");
        assert_eq!(b.read_bytes, l1.len() as u64, "1 回目は全体を読むはず");
        assert!(
            t.last_brief
                .as_deref()
                .is_some_and(|x| x.ends_with("brief-a.md")),
            "brief を拾うはず"
        );
        let half = &l2[..10];
        fs::OpenOptions::new()
            .append(true)
            .open(&f)
            .unwrap()
            .write_all(half.as_bytes())
            .unwrap();
        let t = b.scan(&f, false).expect("読めるはず");
        assert!(
            t.last_brief
                .as_deref()
                .is_some_and(|x| x.ends_with("brief-a.md")),
            "書きかけの行は次の周期に回すはず"
        );
        assert_eq!(
            b.read_bytes,
            (l1.len() + half.len()) as u64,
            "2 回目は追記ぶんだけ読むはず"
        );
        fs::OpenOptions::new()
            .append(true)
            .open(&f)
            .unwrap()
            .write_all(&l2.as_bytes()[10..])
            .unwrap();
        let t = b.scan(&f, false).expect("読めるはず");
        assert_eq!(
            b.read_bytes,
            (l1.len() + half.len() + l2.len()) as u64,
            "書きかけの行の続きから読み直すはず"
        );
        assert!(
            t.last_brief
                .as_deref()
                .is_some_and(|x| x.ends_with("brief-b.md")),
            "出し直した brief に切り替わるはず"
        );
        assert_eq!(
            (t.first_ts, t.last_brief_ts),
            (
                parse_time("2026-01-02T03:00:00Z"),
                parse_time("2026-01-02T03:20:00Z")
            ),
            "最初の時刻と brief の時刻を持つはず"
        );
        fs::write(&f, &l1).unwrap();
        let t = b.scan(&f, false).expect("読めるはず");
        assert!(
            t.last_brief
                .as_deref()
                .is_some_and(|x| x.ends_with("brief-a.md")),
            "縮んだら最初から読み直すはず"
        );
        assert!(
            b.scan(&d.join("missing.jsonl"), false).is_none(),
            "読めなければ None のはず"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn progress_files_are_chosen_by_name_tag() {
        assert_eq!(
            name_tag("5-1700000000000000-progress-wA:p1-ab12.msg"),
            Some("progress"),
            "3 番目が tag のはず"
        );
        assert_eq!(
            name_tag("5-1700000000000000-done-wA:p-1-ab12.msg"),
            Some("done"),
            "from に - があっても tag は 3 番目のはず"
        );
        assert_eq!(name_tag("x.msg"), None, "形の違う名前は tag 無しのはず");
    }

    #[test]
    fn req3_detail_drops_reports_when_transcript_start_is_unknown() {
        let d = tmp("req3");
        let msg = d.join("m.msg");
        fs::write(
            &msg,
            "from: wA:p1\nfrom_socket: /s/a\ntag: report\nsent_at: 2026-01-02T03:04:06Z\n\n\n前の session の報告\n",
        )
        .unwrap();
        let snap = Snapshot {
            files: vec![("m.msg".into(), msg)],
            ..Default::default()
        };
        let row = PaneRow {
            key: "k".into(),
            pane: "wA:p1".into(),
            tag: String::new(),
            status: "working".into(),
            name: String::new(),
            title: String::new(),
            label: String::new(),
            kind: None,
            sid: Some("sess1".into()),
            codex: false,
            tab_id: None,
            ws: (String::new(), String::new()),
            socket: "/s/a".into(),
        };
        let dirs = TranscriptPaths {
            projects: vec![d.clone()],
            ..Default::default()
        };
        let reports = |body: &str| {
            fs::write(d.join("sess1.jsonl"), body).unwrap();
            let det = detail(
                &snap,
                &row,
                &mut Transcripts::default(),
                &mut Briefs::default(),
                &dirs,
            );
            det.events.iter().filter(|e| e.kind == Kind::Report).count()
        };
        assert_eq!(reports(""), 0, "空の transcript には報告を帰属させないはず");
        assert_eq!(
            reports("not json\n"),
            0,
            "壊れた transcript には報告を帰属させないはず"
        );
        assert_eq!(
            reports(&format!("{T}\n")),
            1,
            "開始時刻が分かれば開始以降の報告は帰属するはず"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn parse_msg_headers_and_body() {
        let m = parse_msg(
            "from: wA:p1\ntag: progress\nsent_at: 2026-01-02T03:04:05Z\n\n\n[progress] 調査\n",
        )
        .expect("読めるはず");
        assert_eq!(
            (
                m.from_pane.as_str(),
                m.from_socket.is_none(),
                m.tag.as_str(),
                m.body.as_str()
            ),
            ("wA:p1", true, "progress", "[progress] 調査"),
            "ヘッダと本文の最初の行のはず"
        );
        assert!(
            parse_msg("from: wA:p1\n\nx").is_none(),
            "sent_at が無ければ捨てるはず"
        );
    }
}
