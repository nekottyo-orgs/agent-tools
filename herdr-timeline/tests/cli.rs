// CLI を偽の herdr (fixture の JSON を出す sh script) と一時ディレクトリの projects / runs で動かす。
// 値はすべて架空 (pane id は wA:p1 形、session id は架空の UUID)。

use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const SID1: &str = "0b5c1f3e-7a2d-4c9e-8f10-000000000001";
const SID2: &str = "0b5c1f3e-7a2d-4c9e-8f10-000000000002";
const SID_MISSING: &str = "0b5c1f3e-7a2d-4c9e-8f10-00000000dead";

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fx {
    dir: PathBuf,
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn pane(id: &str, status: &str, sid: &str, tab: &str, ws: &str, label: &str) -> serde_json::Value {
    json!({"pane_id": id, "agent_status": status, "agent_session": {"value": sid}, "tab_id": tab, "workspace_id": ws,
           "label": label, "terminal_title_stripped": format!("{label} の title"), "agent": "claude"})
}

impl Fx {
    fn new(name: &str) -> Fx {
        // テストは並列に走るので、同じ名前でも衝突しないよう連番を足す
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "herdr-timeline-cli-{name}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let fx = Fx { dir };
        let panes = json!({"result": {"panes": [
            pane("wA:p1", "blocked", SID1, "tA:1", "wA", "調査"),
            pane("wA:p2", "working", SID2, "tA:2", "wA", "実装"),
            pane("wA:p3", "idle", "../x", "tA:3", "wA", "検算"),
            pane("wB:p1", "done", SID_MISSING, "tB:1", "wB", "片付け"),
        ]}});
        let wss = json!({"result": {"workspaces": [
            {"workspace_id": "wA", "label": "作業\u{1b}[31m場", "number": 1},
            {"workspace_id": "wB", "label": "別件", "number": 2},
        ]}});
        let tabs = json!({"result": {"tabs": [
            {"tab_id": "tA:1", "label": "調査", "pane_count": 1},
            {"tab_id": "tA:2", "label": "実装", "pane_count": 1},
            {"tab_id": "tA:3", "label": "検算", "pane_count": 1},
            {"tab_id": "tB:1", "label": "片付け", "pane_count": 1},
        ]}});
        fx.write("herdr/panes.json", &panes.to_string());
        fx.write("herdr/workspaces.json", &wss.to_string());
        fx.write("herdr/tabs.json", &tabs.to_string());
        fx.write(
            "herdr/herdr.sh",
            "d=$(dirname \"$0\")\ncase \"$1 $2\" in\n  'pane list') cat \"$d/panes.json\" ;;\n  'workspace list') cat \"$d/workspaces.json\" ;;\n  'tab list') cat \"$d/tabs.json\" ;;\n  *) exit 1 ;;\nesac\n",
        );
        fs::create_dir_all(fx.dir.join("projects/proj")).unwrap();
        fx.write("runs/r1/run.md", "# run\norchestrator_pane: wA:p2\n");
        fx
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    fn write(&self, rel: &str, s: &str) {
        let p = self.path(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    fn sock(&self) -> String {
        self.path("herdr/herdr.sock").to_string_lossy().into_owned()
    }

    fn transcript(&self, sid: &str, lines: &[serde_json::Value]) {
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        self.write(&format!("projects/proj/{sid}.jsonl"), &body);
    }

    /// inbox の msg。sock が None なら from_socket の無い旧形式
    fn msg(&self, rel: &str, from: &str, sock: Option<&str>, tag: &str, at: &str, body: &str) {
        let s = sock
            .map(|s| format!("from_socket: {s}\n"))
            .unwrap_or_default();
        self.write(
            rel,
            &format!("from: {from}\n{s}tag: {tag}\nsent_at: {at}\n\n{body}\n"),
        );
    }

    /// 止まらないことも確かめるため、10 秒で打ち切る
    fn run(&self, args: &[&str]) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_herdr-timeline"))
            .args(args)
            .env(
                "HERDR_TIMELINE_HERDR",
                format!("sh {}", self.path("herdr/herdr.sh").display()),
            )
            .env("HERDR_TIMELINE_HERDR_DIR", self.path("herdr"))
            .env("HERDR_SOCKET_PATH", self.sock())
            .env("HERDR_TIMELINE_PROJECTS", self.path("projects"))
            .env("HERDR_TIMELINE_RUNS_DIR", self.path("runs"))
            .env("NO_COLOR", "1")
            .env_remove("FORCE_COLOR")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while child.try_wait().unwrap().is_none() {
            if start.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                panic!("10 秒で終わらなかった: {args:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        child.wait_with_output().unwrap()
    }
}

fn user(at: &str, content: &str) -> serde_json::Value {
    json!({"type": "user", "timestamp": at, "message": {"role": "user", "content": content}})
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn sh(args: &[&str]) -> Output {
    Fx::new("args").run(args)
}

#[test]
fn once_shows_three_sections_without_controls() {
    let fx = Fx::new("once");
    for i in 0..3 {
        fx.msg(
            &format!("runs/r1/inbox/new/{i}.msg"),
            "wA:p1",
            Some(&fx.sock()),
            "done",
            "2026-01-02T03:10:00Z",
            "x",
        );
    }
    let o = fx.run(&["--once", "--width", "100"]);
    let out = stdout(&o);
    assert_eq!(o.status.code(), Some(0), "--once は 0 で終わるはず: {out}");
    for h in ["■ 要対応 2", "■ 作業中 1", "■ done / idle"] {
        assert!(out.contains(h), "見出し {h} が出るはず: {out}");
    }
    assert!(
        out.contains("▲ 窓口 作業場 / 実装  未処理の報告 3 件"),
        "未処理の報告がある run は要対応に出て窓口は名前で呼ぶはず: {out}"
    );
    assert!(
        out.contains("実装 の title"),
        "ゴールが無ければ title を要約にするはず: {out}"
    );
    assert!(
        !out.contains('\u{1b}'),
        "外部入力の制御文字は出さないはず: {out:?}"
    );
}

#[test]
fn req1_unreadable_transcript_does_not_stop() {
    let fx = Fx::new("req1");
    fx.transcript(SID2, &[user("2026-01-02T03:00:00Z", "読めないはずの指示")]);
    let locked = fx.path(&format!("projects/proj/{SID2}.jsonl"));
    // SAFETY: geteuid は引数を取らず失敗しない
    if unsafe { libc::geteuid() } != 0 {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let o = fx.run(&["--timeline", "wA:p2"]);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            matches!(o.status.code(), Some(0 | 1)),
            "権限の無い transcript でも 0 か 1 で止まるはず: {o:?}"
        );
        assert!(
            !stdout(&o).contains("読めないはずの指示"),
            "読めない transcript のイベントは 0 件のはず"
        );
    } else {
        eprintln!("root で動いているので権限のケースは飛ばす");
    }
    let o = fx.run(&["--timeline", "wB:p1"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "存在しない transcript でも 0 で終わるはず: {o:?}"
    );
    assert!(
        stdout(&o).contains("transcript が見つからない"),
        "transcript が無いことを 1 行で出すはず: {}",
        stdout(&o)
    );
    assert_eq!(
        fx.run(&["--timeline", "wZ:p9"]).status.code(),
        Some(1),
        "pane が無ければ 1 のはず"
    );
}

#[test]
fn req2_inbox_outside_new_and_cur_is_ignored() {
    let fx = Fx::new("req2");
    fx.transcript(SID1, &[user("2026-01-02T03:00:00Z", "最初の指示")]);
    fx.msg(
        "runs/r1/inbox/tmp/a.msg",
        "wA:p1",
        Some(&fx.sock()),
        "done",
        "2026-01-02T03:10:00Z",
        "書きかけの報告",
    );
    let out = stdout(&fx.run(&["--timeline", "wA:p1"]));
    assert!(
        out.contains("最初の指示"),
        "transcript の指示は出るはず: {out}"
    );
    assert!(
        !out.contains("書きかけの報告"),
        "inbox/tmp の msg は出ないはず: {out}"
    );
    fs::rename(fx.path("runs/r1/inbox/tmp"), fx.path("runs/r1/inbox/cur")).unwrap();
    let out = stdout(&fx.run(&["--timeline", "wA:p1"]));
    assert!(
        out.contains("書きかけの報告"),
        "同じ msg を cur に置けば出るはず: {out}"
    );
}

#[test]
fn req3_attribution_by_socket_and_session_start() {
    let fx = Fx::new("req3");
    fx.transcript(SID1, &[user("2026-01-02T03:00:00Z", "最初の指示")]);
    let sock = fx.sock();
    let other = fx
        .path("elsewhere/herdr.sock")
        .to_string_lossy()
        .into_owned();
    fx.msg(
        "runs/r1/inbox/new/1.msg",
        "wA:p1",
        Some(&sock),
        "done",
        "2026-01-02T03:10:00Z",
        "正しい報告",
    );
    fx.msg(
        "runs/r1/inbox/new/2.msg",
        "wA:p1",
        Some(&other),
        "done",
        "2026-01-02T03:11:00Z",
        "別socketの報告",
    );
    fx.msg(
        "runs/r1/inbox/new/3.msg",
        "wA:p1",
        None,
        "done",
        "2026-01-02T03:12:00Z",
        "旧形式の報告",
    );
    fx.msg(
        "runs/r1/inbox/cur/4.msg",
        "wA:p1",
        Some(&sock),
        "done",
        "2026-01-02T02:59:59Z",
        "開始前の報告",
    );
    let out = stdout(&fx.run(&["--timeline", "wA:p1"]));
    assert!(
        out.contains("正しい報告"),
        "socket と pane が一致する msg は出るはず: {out}"
    );
    for bad in ["別socketの報告", "旧形式の報告", "開始前の報告"] {
        assert!(!out.contains(bad), "{bad} は出ないはず: {out}");
    }
}

#[test]
fn req6_bad_arguments_exit_2() {
    for args in [
        &["--timeline"][..],
        &["--timeline", "--once"],
        &["--width", "0"],
        &["--interval", "abc"],
        &["--interval", "86401"],
        &["--width", "1001"],
        &["--interval", "18446744073709551615"],
        &["--bogus"],
        &["--runs-dir"],
    ] {
        let o = sh(args);
        assert_eq!(o.status.code(), Some(2), "{args:?} は exit 2 のはず");
        assert!(
            String::from_utf8_lossy(&o.stderr).contains("使い方"),
            "{args:?} は stderr に使い方を出すはず"
        );
    }
}

#[test]
fn req7_help_and_readme_describe_scope_exit_codes_and_keys() {
    let o = sh(&["-h"]);
    assert_eq!(o.status.code(), Some(0), "-h は 0 のはず");
    let help = stdout(&o);
    let readme = include_str!("../README.md");
    for (doc, needles) in [
        (
            "-h",
            [
                "--timeline",
                "transcript",
                "inbox",
                "書き込まない",
                "終了コード",
                "Enter",
                "\n  t ",
                "\n  q ",
            ],
        ),
        (
            "README",
            [
                "--timeline",
                "transcript",
                "inbox",
                "書き込まない",
                "終了コード",
                "`Enter`",
                "`t`",
                "`q`",
            ],
        ),
    ] {
        let text = if doc == "-h" { help.as_str() } else { readme };
        for n in needles {
            assert!(text.contains(n), "{doc} に {n:?} が載っているはず");
        }
    }
}

#[test]
fn req8_invalid_session_id_has_no_transcript() {
    let fx = Fx::new("req8");
    // ../x を join すると projects/proj/../x.jsonl = projects/x.jsonl を指してしまう
    fx.write(
        "projects/x.jsonl",
        &format!("{}\n", user("2026-01-02T03:00:00Z", "漏れた指示")),
    );
    let o = fx.run(&["--timeline", "wA:p3"]);
    let out = stdout(&o);
    assert_eq!(
        o.status.code(),
        Some(0),
        "弾いた session_id でも 0 で終わるはず"
    );
    assert!(
        out.contains("transcript が見つからない"),
        "弾いた session_id の pane は transcript 無しのはず: {out}"
    );
    assert!(
        !out.contains("漏れた指示"),
        "projects の外れのファイルを読まないはず: {out}"
    );
}

#[test]
fn req9_reissued_brief_is_used() {
    let fx = Fx::new("req9");
    let first = fx.path("briefs/brief-first.md");
    let second = fx.path("briefs/brief-second.md");
    fx.write(
        "briefs/brief-first.md",
        "---\nsteps: [前半, 後半]\n---\n# 依頼: 最初のゴール\n",
    );
    fx.write(
        "briefs/brief-second.md",
        "---\nsteps: [調査, 対応表, 設計案]\n---\n# 依頼: 後のゴール\n",
    );
    let before = [user(
        "2026-01-02T03:00:00Z",
        &format!("{} を読んで進めて", first.display()),
    )];
    fx.transcript(SID1, &before);
    let sock = fx.sock();
    // 名前は <prio>-<micros>-<tag>-<from>-<rand>.msg。pane 行は名前の tag で progress を選ぶ
    // 後の brief を指示する前の progress は数えない
    fx.msg(
        "runs/r1/inbox/cur/5-1-progress-wA:p1-a.msg",
        "wA:p1",
        Some(&sock),
        "progress",
        "2026-01-02T03:10:00Z",
        "[progress] 調査, 対応表, 設計案",
    );
    let out = stdout(&fx.run(&["--once", "--width", "200"]));
    assert!(
        out.contains("最初のゴール  0/2 前半"),
        "出し直す前は最初の brief を pane 行に出すはず: {out}"
    );
    let mut after = before.to_vec();
    after.push(user(
        "2026-01-02T03:20:00Z",
        &format!("方針を変えた。{} を読んで", second.display()),
    ));
    after.push(user("2026-01-02T03:30:00Z", "急いで"));
    fx.transcript(SID1, &after);
    fx.msg(
        "runs/r1/inbox/cur/5-2-progress-wA:p1-b.msg",
        "wA:p1",
        Some(&sock),
        "progress",
        "2026-01-02T03:25:00Z",
        "[progress] 調査, 余談",
    );
    let out = stdout(&fx.run(&["--once", "--width", "200"]));
    assert!(
        out.contains("後のゴール  1/3 対応表"),
        "出し直した brief と指示以降の progress を pane 行に出すはず: {out}"
    );
    let out = stdout(&fx.run(&["--timeline", "wA:p1", "--width", "100"]));
    assert!(
        out.contains("ゴール  後のゴール"),
        "後の brief の H1 を使うはず: {out}"
    );
    assert!(
        out.contains("step 1/3  ✓調査 ・対応表 ・設計案  (brief に無い step 1 件)"),
        "後の brief の steps と、指示以降の progress を使うはず: {out}"
    );
}

#[test]
fn pane_rows_show_goal_of_unexpanded_panes() {
    let fx = Fx::new("rows");
    let brief = fx.path("briefs/brief-impl.md");
    fx.write(
        "briefs/brief-impl.md",
        "---\nsteps: [設計, 実装]\n---\n# 依頼: 実装する\n",
    );
    fx.transcript(
        SID2,
        &[user(
            "2026-01-02T03:00:00Z",
            &format!("{} を読んで", brief.display()),
        )],
    );
    let out = stdout(&fx.run(&["--once", "--width", "200"]));
    assert!(
        out.contains("実装する  進捗未報告"),
        "progress が無ければ進捗未報告を出すはず: {out}"
    );
    // tag が progress でも、名前の tag が違う msg は pane 行では開かない
    fx.msg(
        "runs/r1/inbox/cur/5-1-done-wA:p2-a.msg",
        "wA:p2",
        Some(&fx.sock()),
        "progress",
        "2026-01-02T03:05:00Z",
        "[progress] 設計",
    );
    assert!(
        stdout(&fx.run(&["--once", "--width", "200"])).contains("実装する  進捗未報告"),
        "名前の tag が progress でない msg は数えないはず"
    );
    fx.msg(
        "runs/r1/inbox/cur/5-2-progress-wA:p2-b.msg",
        "wA:p2",
        Some(&fx.sock()),
        "progress",
        "2026-01-02T03:10:00Z",
        "[progress] 設計",
    );
    let out = stdout(&fx.run(&["--once", "--width", "200"]));
    assert!(
        out.contains("実装する  1/2 実装"),
        "展開していない pane の行にゴールと N/M と次の step を出すはず: {out}"
    );
}

#[test]
fn req2_transcript_exclusions_hide_and_removing_them_shows() {
    for key in ["isMeta", "isSidechain", "isCompactSummary"] {
        let fx = Fx::new("req2-transcript");
        let mut hidden = user("2026-01-02T03:01:00Z", "除外の行");
        hidden[key] = json!(true);
        fx.transcript(
            SID1,
            &[user("2026-01-02T03:00:00Z", "最初の指示"), hidden.clone()],
        );
        let out = stdout(&fx.run(&["--timeline", "wA:p1"]));
        assert!(
            !out.contains("除外の行"),
            "{key} だけが付いた行は出ないはず: {out}"
        );
        hidden[key] = json!(false);
        fx.transcript(SID1, &[user("2026-01-02T03:00:00Z", "最初の指示"), hidden]);
        let out = stdout(&fx.run(&["--timeline", "wA:p1"]));
        assert!(
            out.contains("除外の行"),
            "{key} を外した同じ行は出るはず: {out}"
        );
    }
}

#[test]
fn req4_pastes_and_skip_prefixes_with_leading_whitespace() {
    let fx = Fx::new("req4");
    fx.transcript(
        SID1,
        &[
            user("2026-01-02T03:00:00Z", "\n  <pasted_content>\n貼った中身\n</pasted_content>"),
            user("2026-01-02T03:02:00Z", "\n\nStop hook feedback: 出さない"),
            json!({"type": "pr-link", "timestamp": "2026-01-02T03:03:00Z", "prUrl": "u1", "prRepository": "o/r", "prNumber": 7}),
        ],
    );
    let out = stdout(&fx.run(&["--timeline", "wA:p1"]));
    assert!(
        out.contains("貼った中身") && out.contains("o/r#7"),
        "先頭に改行の付いた貼り付けの中身と PR が出るはず: {out}"
    );
    assert!(
        !out.contains("出さない"),
        "先頭に改行の付いた skip 前置きは出ないはず: {out}"
    );
}
