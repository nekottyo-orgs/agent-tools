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
            "d=$(dirname \"$0\")\ncase \"$1 $2\" in\n  'pane list') cat \"$d/panes.json\" ;;\n  'workspace list') cat \"$d/workspaces.json\" ;;\n  'tab list') cat \"$d/tabs.json\" ;;\n  'pane read') echo \"$3\" >> \"$d/reads.log\"; echo \"$4 $5\" >> \"$d/reads.args\"; [ -f \"$d/screen-$3\" ] && tail -n \"$5\" \"$d/screen-$3\" ;;\n  *) exit 1 ;;\nesac\n",
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
        self.run_with(args, true)
    }

    /// runs が false なら HERDR_TIMELINE_RUNS_DIR を渡さない (runs-dir 未指定)
    fn run_with(&self, args: &[&str], runs: bool) -> Output {
        self.run_env(args, runs, &[])
    }

    /// extra は追加の環境変数 (watchers の state_dir・ps の差し替え用)
    fn run_env(&self, args: &[&str], runs: bool, extra: &[(&str, String)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_herdr-timeline"));
        // 既定では実 HOME の state と実 ps に触れないよう、存在しない state_dir と何も出さない stub ps に向ける。extra で上書きできる
        self.write("ps-none/ps.sh", "#!/bin/sh\nexit 0\n");
        std::fs::set_permissions(
            self.path("ps-none/ps.sh"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        for k in &WATCH_ENV[..3] {
            cmd.env(k, self.path("state-none").join(k));
        }
        cmd.env(WATCH_ENV[3], self.path("ps-none/ps.sh"));
        cmd.envs(extra.iter().map(|(k, v)| (*k, v)));
        if !runs {
            cmd.env_remove("HERDR_TIMELINE_RUNS_DIR");
        }
        let mut child = cmd
            .args(args)
            .env(
                "HERDR_TIMELINE_HERDR",
                format!("sh {}", self.path("herdr/herdr.sh").display()),
            )
            .env("HERDR_TIMELINE_HERDR_DIR", self.path("herdr"))
            .env("HERDR_SOCKET_PATH", self.sock())
            .env("HERDR_TIMELINE_PROJECTS", self.path("projects"))
            .env("CODEX_HOME", self.path("codex"))
            .envs(runs.then(|| ("HERDR_TIMELINE_RUNS_DIR", self.path("runs"))))
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
        out.contains("▲ 窓口 稼働中 作業場 / 実装  未処理の報告 3 件"),
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
fn run_alerts_show_owner_state_without_hiding_unread_messages() {
    let fx = Fx::new("run-alerts");
    for socket in [
        "herdr.sock",
        "sessions/empty/herdr.sock",
        "sessions/failed/herdr.sock",
        "sessions/invalid/herdr.sock",
        "sessions/null/herdr.sock",
    ] {
        fx.write(&format!("herdr/{socket}"), "");
    }
    let herdr = fs::read_to_string(fx.path("herdr/herdr.sh")).unwrap();
    fx.write(
        "herdr/herdr.sh",
        &format!(
            r#"if [ "$1 $2" = 'pane list' ]; then
  case "$HERDR_SOCKET_PATH" in
    */empty/herdr.sock) echo '{{"result":{{"panes":[]}}}}'; exit 0 ;;
    */failed/herdr.sock) exit 1 ;;
    */invalid/herdr.sock) echo '{{"result":{{"panes":[{{"pane_id":"wA:p2","agent_status":"working"}},{{"pane_id":" ","agent_status":"blocked","label":"架空の要対応"}},{{}}]}}}}'; exit 0 ;;
    */null/herdr.sock) echo '{{"result":{{"panes":null}}}}'; exit 0 ;;
  esac
fi
{herdr}"#,
        ),
    );

    let cases = [
        ("r1", "herdr.sock", "窓口 稼働中 作業場 / 実装"),
        ("absent", "sessions/empty/herdr.sock", "窓口不在 absent"),
        ("failed", "sessions/failed/herdr.sock", "状態未確認 failed"),
        (
            "invalid",
            "sessions/invalid/herdr.sock",
            "状態未確認 invalid",
        ),
        ("null", "sessions/null/herdr.sock", "状態未確認 null"),
        (
            "unseen\u{1b}[31m",
            "sessions/unseen/herdr.sock",
            "状態未確認 unseen",
        ),
        ("closed", "herdr.sock", "終了済み closed"),
    ];
    for (id, socket, _) in cases {
        let prefix = if id == "closed" { "closed_" } else { "" };
        let mut md = format!("{prefix}orchestrator_pane: wA:p2\n");
        if !socket.is_empty() {
            let path = fx.path(&format!("herdr/{socket}"));
            md.push_str(&format!(
                "{prefix}orchestrator_socket: {}\n",
                path.display()
            ));
        }
        fx.write(&format!("runs/{id}/run.md"), &md);
        let msg = format!("runs/{id}/inbox/new/0-request.msg");
        fx.msg(
            &msg,
            "wA:p2",
            Some(&fx.sock()),
            "request",
            "2026-01-02T03:10:00Z",
            "引き継ぐ依頼",
        );
    }
    let o = fx.run(&["--once", "--width", "300"]);
    let out = stdout(&o);
    assert_eq!(o.status.code(), Some(0), "{out}");
    assert!(!out.contains('\u{1b}'), "{out:?}");
    for (id, _, state) in cases {
        let msg = format!("runs/{id}/inbox/new/0-request.msg");
        assert!(
            out.contains(&format!("{state}  未処理の報告 1 件")),
            "run の状態と未処理件数が出るはず: {out}"
        );
        assert!(
            fs::read_to_string(fx.path(&msg))
                .unwrap()
                .ends_with("引き継ぐ依頼\n"),
            "未処理の依頼は残るはず"
        );
    }
    let warning = out
        .lines()
        .find(|line| line.starts_with("!! 取得不全:"))
        .expect("不完全な一覧の診断が出るはず");
    let v = json_of(&fx.run(&["--json"]));
    assert_eq!(v.as_object().unwrap().len(), 3, "{v}");
    assert!(v["watchers"].is_array(), "{v}");
    let warnings = v["warnings"].as_array().unwrap();
    for diagnostic in [
        "failed: herdr pane list が終了コード 1 で失敗",
        "invalid: herdr pane list に有効な pane_id のない pane がある",
        "null: herdr pane list の panes が配列でない",
    ] {
        assert!(warning.contains(diagnostic), "{warning}");
        assert!(warnings.iter().any(|w| w == diagnostic), "{v}");
    }
    assert_eq!(warnings.len(), 3, "正常な空一覧に警告は出ないはず: {v}");
    let panes = v["panes"].as_array().unwrap();
    let reads = fs::read_to_string(fx.path("herdr/reads.log")).unwrap();
    assert_eq!(
        (
            out.contains("架空の要対応"),
            panes.iter().any(|p| p["pane_id"] == ""),
            reads.lines().any(str::is_empty),
        ),
        (false, false, false),
        "無効な ID は画面・JSON・pane read のいずれにも渡らないはず"
    );
    assert!(panes.iter().any(|p| p["key"] == "invalid/wA:p2"), "{v}");
    assert!(out.contains("invalid/wA:p2"), "{out}");
    assert_eq!(jpane(&v, "wA:p1")["key"], "default/wA:p1", "{v}");
    assert!(out.contains("default/wA:p1"), "{out}");
}

#[test]
fn legacy_run_alerts_only_match_the_default_session() {
    let fx = Fx::new("legacy-run-alerts");
    fx.write("herdr/sessions/workers/herdr.sock", "");
    fx.write("runs/r1/inbox/new/0-request.msg", "");

    let absent_default = fx.run(&["--once", "--width", "100"]);
    assert!(
        stdout(&absent_default).contains("状態未確認 r1  未処理の報告 1 件"),
        "別セッションの同じ pane ID は窓口ではないはず: {}",
        stdout(&absent_default)
    );

    fx.write("herdr/herdr.sock", "");
    let default = fx.run(&["--once", "--width", "100"]);
    assert!(
        stdout(&default).contains("窓口 稼働中 作業場 / 実装  未処理の報告 1 件"),
        "default の窓口を表示するはず: {}",
        stdout(&default)
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
fn resident_panes_hidden_unless_show_resident() {
    let fx = Fx::new("resident");
    let mut panes: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fx.path("herdr/panes.json")).unwrap())
            .unwrap();
    let list = panes["result"]["panes"].as_array_mut().unwrap();
    list.push(pane("wA:p8", "working", SID2, "tA:2", "wA", "watch-worker"));
    list.push(pane("wA:p9", "working", SID2, "tA:2", "wA", "impl-task"));
    list.push(pane(
        "wA:p7",
        "blocked",
        SID2,
        "tA:2",
        "wA",
        "review-submit",
    ));
    fx.write("herdr/panes.json", &panes.to_string());
    let hidden = stdout(&fx.run(&["--once", "--width", "100"]));
    assert!(
        !hidden.contains("watch-worker の title"),
        "常駐の pane は既定で出ないはず: {hidden}"
    );
    assert!(
        hidden.contains("impl-task の title"),
        "常駐でない pane は出るはず: {hidden}"
    );
    assert!(
        hidden.contains("review-submit の title"),
        "blocked の常駐 pane は要対応なので隠さないはず: {hidden}"
    );
    let tl = fx.run(&["--timeline", "wA:p8"]);
    assert_eq!(
        tl.status.code(),
        Some(0),
        "--timeline は常駐 pane も引けるはず"
    );
    let shown = stdout(&fx.run(&["--once", "--width", "100", "--show-resident"]));
    assert!(
        shown.contains("watch-worker の title") && shown.contains("impl-task の title"),
        "--show-resident では常駐も出るはず: {shown}"
    );
}

#[test]
fn dispatched_panes_hidden_unless_in_ledger_or_blocked() {
    let fx = Fx::new("dispatched");
    let mut panes: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fx.path("herdr/panes.json")).unwrap())
            .unwrap();
    let list = panes["result"]["panes"].as_array_mut().unwrap();
    list.push(pane("wA:p5", "working", SID2, "tA:2", "wA", "repo-123"));
    list.push(pane(
        "wA:p6",
        "working",
        SID2,
        "tA:2",
        "wA",
        "repo-bundle-45",
    ));
    list.push(pane("wA:p7", "blocked", SID2, "tA:2", "wA", "repo-67"));
    list.push(pane("wA:p9", "working", SID2, "tA:2", "wA", "impl-task"));
    fx.write("herdr/panes.json", &panes.to_string());
    fx.msg(
        "runs/r1/inbox/new/1.msg",
        "wA:p6",
        Some(&fx.sock()),
        "done",
        "2026-01-02T03:10:00Z",
        "報告",
    );
    let a = stdout(&fx.run(&["--once", "--width", "100"]));
    assert!(
        !a.contains("repo-123 の title"),
        "run に属さない dispatch の pane は出ないはず: {a}"
    );
    assert!(
        a.contains("repo-bundle-45 の title"),
        "inbox に msg がある pane は同じ label でも出るはず: {a}"
    );
    assert!(a.contains("repo-67 の title"), "blocked は出るはず: {a}");
    assert!(a.contains("impl-task の title"), "{a}");
    let shown = stdout(&fx.run(&["--once", "--width", "100", "--show-resident"]));
    assert!(
        shown.contains("repo-123 の title"),
        "--show-resident では出るはず: {shown}"
    );
    let none = stdout(&fx.run_with(&["--once", "--width", "100"], false));
    assert!(
        none.contains("repo-123 の title"),
        "runs-dir 無しなら台帳が読めないので外さないはず: {none}"
    );
}

fn dispatched_fx(name: &str) -> Fx {
    let fx = Fx::new(name);
    let mut panes: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fx.path("herdr/panes.json")).unwrap())
            .unwrap();
    let list = panes["result"]["panes"].as_array_mut().unwrap();
    list.push(pane("wA:p5", "working", SID2, "tA:2", "wA", "repo-123"));
    fx.write("herdr/panes.json", &panes.to_string());
    fx
}

#[test]
fn dispatched_pane_shown_when_orchestrator_pane() {
    let fx = dispatched_fx("dispatched-orch");
    fx.write("runs/r1/run.md", "# run\norchestrator_pane: wA:p5\n");
    let a = stdout(&fx.run(&["--once", "--width", "100"]));
    assert!(
        a.contains("repo-123 の title"),
        "窓口の pane は dispatch の形でも出るはず: {a}"
    );
}

#[test]
fn dispatched_pane_shown_when_orchestrator_socket_matches() {
    let fx = dispatched_fx("dispatched-orch-sock");
    fx.write(
        "runs/r1/run.md",
        &format!(
            "# run\norchestrator_pane: wA:p5\norchestrator_socket: {}\n",
            fx.sock()
        ),
    );
    let a = stdout(&fx.run(&["--once", "--width", "100"]));
    assert!(
        a.contains("repo-123 の title"),
        "orchestrator_socket が一致する窓口は出るはず: {a}"
    );
}

#[test]
fn dispatched_pane_hidden_when_orchestrator_socket_differs() {
    let fx = dispatched_fx("dispatched-orch-other");
    fx.write(
        "runs/r1/run.md",
        "# run\norchestrator_pane: wA:p5\norchestrator_socket: /other/herdr.sock\n",
    );
    let a = stdout(&fx.run(&["--once", "--width", "100"]));
    assert!(
        !a.contains("repo-123 の title"),
        "別 socket の窓口は pane id が同じでも外れるはず: {a}"
    );
}

#[test]
fn dispatched_pane_hidden_when_msg_from_other_socket() {
    let fx = dispatched_fx("dispatched-sock");
    fx.msg(
        "runs/r1/inbox/new/1.msg",
        "wA:p5",
        Some("/other/herdr.sock"),
        "done",
        "2026-01-02T03:10:00Z",
        "報告",
    );
    let a = stdout(&fx.run(&["--once", "--width", "100"]));
    assert!(
        !a.contains("repo-123 の title"),
        "別 socket の msg は帰属しないので外れるはず: {a}"
    );
}

#[test]
fn hidden_orchestrator_pane_keeps_its_name_in_run_row() {
    let fx = Fx::new("hidden-orch-name");
    let rd = |p: &str| {
        serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(fx.path(p)).unwrap())
            .unwrap()
    };
    let (mut panes, mut tabs) = (rd("herdr/panes.json"), rd("herdr/tabs.json"));
    // 窓口 (wA:p5) は常駐 label で一覧から隠れるが、窓口行の名前には残るはず
    panes["result"]["panes"].as_array_mut().unwrap().push(pane(
        "wA:p5",
        "working",
        SID2,
        "tA:9",
        "wA",
        "watch-worker",
    ));
    tabs["result"]["tabs"]
        .as_array_mut()
        .unwrap()
        .push(json!({"tab_id": "tA:9", "label": "窓口役", "pane_count": 1}));
    fx.write("herdr/panes.json", &panes.to_string());
    fx.write("herdr/tabs.json", &tabs.to_string());
    fx.write("runs/r1/run.md", "# run\norchestrator_pane: wA:p5\n");
    fx.msg(
        "runs/r1/inbox/new/1.msg",
        "wA:p2",
        Some(&fx.sock()),
        "done",
        "2026-01-02T03:10:00Z",
        "報告",
    );
    let a = stdout(&fx.run(&["--once", "--width", "100"]));
    assert!(
        !a.contains("watch-worker の title"),
        "常駐の窓口 pane は一覧から隠れるはず: {a}"
    );
    assert!(
        a.contains("▲ 窓口 稼働中 作業場 / 窓口役"),
        "隠した窓口 pane も名前で出るはず (pane id だけにならない): {a}"
    );
}

#[test]
fn dispatched_pane_shown_when_runs_dir_missing() {
    let fx = dispatched_fx("dispatched-nodir");
    std::fs::remove_dir_all(fx.path("runs")).unwrap();
    let a = stdout(&fx.run(&["--once", "--width", "100"]));
    assert!(
        a.contains("repo-123 の title"),
        "存在しない runs-dir なら台帳が読めないので外さないはず: {a}"
    );
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
                "--show-resident",
                "transcript",
                "inbox",
                "書き込まない",
                "終了コード",
                "Enter",
                "\n  Esc / q ",
                "HERDR_TIMELINE_CALLER",
            ],
        ),
        (
            "README",
            [
                "--timeline",
                "--show-resident",
                "transcript",
                "inbox",
                "書き込まない",
                "終了コード",
                "`Enter`",
                "`Esc`",
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

// --json 用。常駐・dispatch・通常の pane を混ぜる。RESIDENT_LABELS は期待値に使わず、ここに独立した一覧を書く
const RESIDENT: [&str; 5] = [
    "watch-my-pr-reviews",
    "watch-other-pr-reviews",
    "watch-worker",
    "review-submit",
    "Cue回答の監視",
];

fn mixed_fx(name: &str) -> Fx {
    let fx = Fx::new(name);
    let mut panes: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fx.path("herdr/panes.json")).unwrap()).unwrap();
    let list = panes["result"]["panes"].as_array_mut().unwrap();
    list.push(pane("wA:p5", "working", SID2, "tA:2", "wA", "repo-123"));
    list.push(pane(
        "wA:p6",
        "working",
        SID2,
        "tA:2",
        "wA",
        "repo-bundle-45",
    ));
    list.push(pane("wA:p7", "blocked", SID2, "tA:2", "wA", "repo-67"));
    list.push(pane("wA:p8", "working", SID2, "tA:2", "wA", RESIDENT[2]));
    list.push(pane("wA:p9", "blocked", SID2, "tA:2", "wA", RESIDENT[0]));
    list.push(pane("wA:p10", "blocked", SID2, "tA:2", "wA", RESIDENT[3]));
    list.push(pane("wA:p11", "idle", SID2, "tA:2", "wA", RESIDENT[4]));
    list.push(pane("wB:p2", "unknown", SID2, "tB:1", "wB", "補助"));
    fx.write("herdr/panes.json", &panes.to_string());
    // wA:p6 は台帳に属す (inbox の msg が帰属する)
    fx.msg(
        "runs/r1/inbox/new/1.msg",
        "wA:p6",
        Some(&fx.sock()),
        "done",
        "2026-01-02T03:10:00Z",
        "報告",
    );
    fx
}

fn json_of(o: &Output) -> serde_json::Value {
    let s = stdout(o);
    assert!(
        s.ends_with("}\n") && !s.ends_with("\n\n"),
        "JSON と改行 1 つだけのはず: {s}"
    );
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("stdout が JSON でない ({e}): {s}"))
}

fn jpane<'a>(v: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    v["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pane_id"] == id)
        .unwrap_or_else(|| panic!("{id} が無い: {v}"))
}

/// --once の出力の (個別の行の pane tag, 畳んだ件数の合計)。個別の行は状態の記号 (× ◐) で始まり、行末が pane tag。
/// 畳んだ行 (done / idle の節) は "<記号> <件数>" の並び
fn once_counts(out: &str) -> (Vec<String>, usize) {
    let mut rows = Vec::new();
    let mut folded = 0;
    let mut in_fold = false;
    for l in out.lines() {
        let t = l.trim_start();
        if t.starts_with('■') {
            in_fold = t.starts_with("■ done");
        } else if in_fold {
            let w: Vec<&str> = t.split_whitespace().collect();
            folded += w
                .windows(2)
                .filter(|p| ["○", "✓", "·"].contains(&p[0]))
                .filter_map(|p| p[1].parse::<usize>().ok())
                .sum::<usize>();
        } else if t.starts_with('×') || t.starts_with('◐') {
            rows.push(t.split_whitespace().last().unwrap().to_string());
        }
    }
    (rows, folded)
}

/// (blocked か working の key の集合, それ以外の数)
fn json_split(v: &serde_json::Value, all: bool) -> (Vec<String>, usize) {
    let mut keys = Vec::new();
    let mut rest = 0;
    for p in v["panes"].as_array().unwrap() {
        if !all && p["hidden"] == true {
            continue;
        }
        match p["status"].as_str().unwrap() {
            "blocked" | "working" => keys.push(p["key"].as_str().unwrap().to_string()),
            _ => rest += 1,
        }
    }
    keys.sort();
    (keys, rest)
}

#[test]
fn json_matches_once_rows_and_folded_counts() {
    let fx = mixed_fx("json-once");
    let v = json_of(&fx.run(&["--json"]));
    assert!(
        v["warnings"].as_array().unwrap().is_empty(),
        "警告は無いはず: {v}"
    );
    // (a) hidden でない blocked / working は --once の個別の行と 1 対 1、(b) 残りは畳んだ件数の合計
    let (mut rows, folded) = once_counts(&stdout(&fx.run(&["--once", "--width", "100"])));
    rows.sort();
    let (keys, rest) = json_split(&v, false);
    assert_eq!(keys, rows, "個別の行の集合が一致するはず");
    assert_eq!(rest, folded, "畳んだ件数の合計が一致するはず");
    // (c) 全 pane の集合は --once --show-resident と等しい
    let (mut rows, folded) = once_counts(&stdout(&fx.run(&[
        "--once",
        "--width",
        "100",
        "--show-resident",
    ])));
    rows.sort();
    let (keys, rest) = json_split(&v, true);
    assert_eq!(
        keys, rows,
        "--show-resident の個別の行は全 pane と一致するはず"
    );
    assert_eq!(
        rest, folded,
        "--show-resident の畳んだ件数は全 pane と一致するはず"
    );
}

#[test]
fn json_marks_resident_and_dispatch_with_owner() {
    let fx = mixed_fx("json-kind");
    let v = json_of(&fx.run(&["--json"]));
    let f = |id: &str| {
        let p = jpane(&v, id);
        (
            p["hidden"].clone(),
            p["hidden_reason"].clone(),
            p["owner"].clone(),
        )
    };
    assert_eq!(
        f("wA:p8"),
        (json!(true), json!("resident"), json!("watch-worker")),
        "常駐は隠すはず"
    );
    assert_eq!(
        f("wA:p11"),
        (json!(true), json!("resident"), json!(null)),
        "watch- 以外の常駐は owner が null のはず"
    );
    assert_eq!(
        f("wA:p9"),
        (json!(false), json!(null), json!("watch-my-pr-reviews")),
        "blocked の常駐は隠さず owner を持つはず"
    );
    assert_eq!(
        f("wA:p5"),
        (json!(true), json!("dispatch"), json!("repo#123")),
        "台帳に属さない dispatch は隠すはず"
    );
    assert_eq!(
        f("wA:p7"),
        (json!(false), json!(null), json!("repo#67")),
        "blocked の dispatch は隠さず owner を持つはず"
    );
    assert_eq!(
        f("wA:p6"),
        (json!(false), json!(null), json!(null)),
        "台帳に属す pane は分類しないはず"
    );
    assert_eq!(
        f("wA:p2"),
        (json!(false), json!(null), json!(null)),
        "通常の pane は何も付かないはず"
    );
    // bundle は owner の repo 略称から落とす
    let fx2 = Fx::new("json-bundle");
    let mut panes: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fx2.path("herdr/panes.json")).unwrap()).unwrap();
    panes["result"]["panes"].as_array_mut().unwrap().push(pane(
        "wA:p6",
        "working",
        SID2,
        "tA:2",
        "wA",
        "repo-bundle-45",
    ));
    fx2.write("herdr/panes.json", &panes.to_string());
    let v2 = json_of(&fx2.run(&["--json"]));
    assert_eq!(
        jpane(&v2, "wA:p6")["owner"],
        json!("repo#45"),
        "-bundle は落とすはず"
    );
    // --show-resident を付けても JSON の hidden は変わらない
    assert_eq!(
        v,
        json_of(&fx.run(&["--json", "--show-resident"])),
        "--show-resident で変わらないはず"
    );
    // 秘匿する項目は出さない
    let s = v.to_string();
    for k in ["socket", "transcript", "session", SID2] {
        assert!(!s.contains(k), "{k} は出さないはず: {s}");
    }
}

#[test]
fn json_dispatch_not_marked_without_readable_runs() {
    let fx = mixed_fx("json-noruns");
    let v = json_of(&fx.run_with(&["--json"], false));
    let p = jpane(&v, "wA:p5");
    assert_eq!(
        (&p["hidden"], &p["owner"]),
        (&json!(false), &json!(null)),
        "run が読めなければ dispatch にしないはず"
    );
}

#[test]
fn json_conflicts_with_timeline_and_implies_once() {
    let fx = Fx::new("json-args");
    let o = fx.run(&["--json", "--timeline", "wA:p1"]);
    assert_eq!(o.status.code(), Some(2), "併用は引数の誤りのはず");
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("使い方"),
        "USAGE を出すはず"
    );
    assert!(o.stdout.is_empty(), "stdout には何も出さないはず");
    // --once なしでも 1 回で終わる (run は 10 秒で打ち切る)
    assert_eq!(
        fx.run(&["--json"]).status.code(),
        Some(0),
        "--once を含意するはず"
    );
}

// 見つかった socket が 1 つで、HERDR_SOCKET_PATH が実在しない socket を指すとき、見つかった server の pane を落とさない。
// fake herdr は HERDR_SOCKET_PATH が実在するときだけ答える
#[test]
fn json_uses_the_only_discovered_socket_when_env_socket_is_absent() {
    let fx = Fx::new("json-one-sock");
    fx.write("herdr/sessions/foo/herdr.sock", "");
    let orig = fs::read_to_string(fx.path("herdr/herdr.sh")).unwrap();
    fx.write(
        "herdr/herdr.sh",
        &format!("[ -e \"$HERDR_SOCKET_PATH\" ] || exit 1\n{orig}"),
    );
    let o = fx.run(&["--json"]);
    let v = json_of(&o);
    assert_eq!(
        v["panes"].as_array().unwrap().len(),
        4,
        "見つかった 1 つの server の pane が出るはず: {v}"
    );
    assert!(
        v["warnings"].as_array().unwrap().is_empty(),
        "warning は出ないはず: {v}"
    );
}

#[test]
fn json_warns_when_pane_list_fails() {
    let fx = Fx::new("json-warn");
    fx.write("herdr/herdr.sh", "exit 1\n");
    let v = json_of(&fx.run(&["--json"]));
    assert!(
        v["panes"].as_array().unwrap().is_empty(),
        "panes は空のはず: {v}"
    );
    assert!(
        !v["warnings"].as_array().unwrap().is_empty(),
        "失敗は warnings に出るはず: {v}"
    );
}

// anomaly。fake の pane read は screen-<pane id> の末尾 --lines 行を返し、呼び出しを reads.log / reads.args に残す
fn anomaly_of(v: &serde_json::Value, id: &str) -> serde_json::Value {
    let ps = v["panes"].as_array().unwrap();
    ps.iter().find(|p| p["pane_id"] == id).unwrap()["anomaly"].clone()
}

fn screen_with(at_line: usize, total: usize) -> String {
    (1..=total)
        .map(|i| {
            if i == at_line {
                "Reconnect failed (3)".to_string()
            } else {
                format!("line {i}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[test]
fn json_anomaly_matches_within_last_20_lines() {
    let fx = Fx::new("json-anomaly");
    // 末尾 20 行の中 (30 行中の 15 行目は外、25 行目は中)
    fx.write("herdr/screen-wA:p2", &screen_with(25, 30));
    fx.write("herdr/screen-wA:p1", &screen_with(10, 30));
    let v = json_of(&fx.run(&["--json"]));
    assert_eq!(anomaly_of(&v, "wA:p2"), "Reconnect failed", "{v}");
    assert!(
        anomaly_of(&v, "wA:p1").is_null(),
        "末尾 20 行の外は null: {v}"
    );
    // herdr へ --lines 20 を渡している
    let args = fs::read_to_string(fx.path("herdr/reads.args")).unwrap();
    assert!(args.lines().all(|l| l == "--lines 20"), "{args}");
}

#[test]
fn json_anomaly_reads_only_working_and_blocked() {
    let fx = Fx::new("json-anomaly-status");
    for id in ["wA:p1", "wA:p2", "wA:p3", "wB:p1"] {
        fx.write(&format!("herdr/screen-{id}"), "Reconnect failed\n");
    }
    let v = json_of(&fx.run(&["--json"]));
    assert_eq!(
        anomaly_of(&v, "wA:p1"),
        "Reconnect failed",
        "blocked は読む: {v}"
    );
    assert_eq!(anomaly_of(&v, "wA:p2"), "Reconnect failed", "{v}");
    assert!(anomaly_of(&v, "wA:p3").is_null(), "idle は null: {v}");
    assert!(anomaly_of(&v, "wB:p1").is_null(), "done は null: {v}");
    let mut reads: Vec<String> = fs::read_to_string(fx.path("herdr/reads.log"))
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    reads.sort();
    assert_eq!(
        reads,
        ["wA:p1", "wA:p2"],
        "done / idle には pane read を呼ばないはず"
    );
}

#[test]
fn json_anomaly_null_without_match_or_when_read_fails() {
    let fx = Fx::new("json-anomaly-none");
    // wA:p1 は一致なし、wA:p2 は screen が無く fake の pane read が失敗する
    fx.write("herdr/screen-wA:p1", "all good\n");
    let o = fx.run(&["--json"]);
    assert!(
        o.status.success(),
        "pane read が失敗しても全体は成功するはず"
    );
    let v = json_of(&o);
    assert!(anomaly_of(&v, "wA:p1").is_null(), "{v}");
    assert!(anomaly_of(&v, "wA:p2").is_null(), "{v}");
    assert!(
        v["warnings"].as_array().unwrap().is_empty(),
        "warnings には積まない: {v}"
    );
}

#[test]
fn json_inbox_counts_attributed_msgs_only() {
    let fx = Fx::new("json-inbox");
    fx.transcript(SID2, &[user("2026-01-02T03:00:00Z", "実装して")]);
    fx.transcript(SID1, &[user("2026-01-02T03:00:00Z", "調べて")]);
    let sock = fx.sock();
    let m = |n: &str, from: &str, s: Option<&str>, at: &str| {
        fx.msg(
            &format!("runs/r1/inbox/new/{n}.msg"),
            from,
            s,
            "report",
            at,
            "報告",
        );
    };
    m("1", "wA:p2", Some(&sock), "2026-01-02T03:10:00Z");
    m("2", "wA:p2", Some(&sock), "2026-01-02T03:20:00Z");
    m(
        "3",
        "wA:p2",
        Some("/nonexistent/other.sock"),
        "2026-01-02T03:30:00Z",
    );
    m("4", "wA:p2", None, "2026-01-02T03:40:00Z");
    m("5", "wA:p2", Some(&sock), "2026-01-02T02:59:59Z");
    let v = json_of(&fx.run(&["--json"]));
    // 2026-01-02T03:20:00Z = 1767324000
    assert_eq!(
        jpane(&v, "wA:p2")["inbox"],
        json!({"count": 2, "last_at": 1767324000}),
        "帰属する msg だけ数えるはず"
    );
    assert_eq!(
        jpane(&v, "wA:p1")["inbox"],
        json!({"count": 0, "last_at": null}),
        "msg が無ければ 0 件のはず"
    );
    assert_eq!(
        jpane(&v, "wA:p3")["inbox"],
        json!(null),
        "transcript が無い pane は null のはず"
    );
    assert!(
        jpane(&v, "wA:p2")["working_secs"]
            .as_i64()
            .is_some_and(|n| n > 0),
        "working は transcript の起点からの経過を持つはず: {v}"
    );
    assert_eq!(
        jpane(&v, "wA:p1")["working_secs"],
        json!(null),
        "working 以外は null のはず"
    );
}

#[test]
fn json_queue_separates_pending_from_delivered() {
    let fx = Fx::new("json-queue");
    let msg = "<pasted_content id=\"a1\">\n[herdr-message] from 窓口 (wA:p1)\n\n続きをお願いします\n</pasted_content>";
    let q = |op: &str, at: &str, reason: Option<&str>| {
        let mut v =
            json!({"type": "queue-operation", "operation": op, "timestamp": at, "content": msg});
        if let Some(r) = reason {
            v["reason"] = r.into();
        }
        v
    };
    let base = [
        user("2026-01-02T03:00:00Z", "実装して"),
        q("enqueue", "2026-01-02T03:01:00Z", None),
    ];
    fx.transcript(SID2, &base);
    // 2026-01-02T03:00:00Z = 1767322800
    let v = json_of(&fx.run(&["--json"]));
    assert_eq!(
        jpane(&v, "wA:p2")["queue"],
        json!({"pending": 1, "oldest_at": 1767322860, "last_input_at": 1767322800}),
        "enqueue だけなら未到達 1 件のはず"
    );
    assert_eq!(
        jpane(&v, "wA:p3")["queue"],
        json!(null),
        "transcript が無い pane は null のはず"
    );
    let mut all = base.to_vec();
    all.push(q(
        "remove",
        "2026-01-02T03:04:00Z",
        Some("absorbed_mid_turn"),
    ));
    fx.transcript(SID2, &all);
    let v = json_of(&fx.run(&["--json"]));
    assert_eq!(
        jpane(&v, "wA:p2")["queue"],
        json!({"pending": 0, "oldest_at": null, "last_input_at": 1767323040}),
        "remove で届いたら pending は 0、last_input_at は remove の時刻のはず"
    );
}

#[test]
fn json_working_secs_never_negative() {
    let fx = Fx::new("json-working-future");
    // 起点が now より後 (now の取得後に入力が届いた状態) を未来の時刻で作る
    fx.transcript(SID2, &[user("2099-01-01T00:00:00Z", "実装して")]);
    let v = json_of(&fx.run(&["--json"]));
    assert_eq!(
        jpane(&v, "wA:p2")["working_secs"],
        json!(0),
        "起点が now より後でも 0 に丸めるはず: {v}"
    );
}

#[test]
fn json_server_and_key_follow_session_count() {
    // 1 session: server は null、key は pane_id
    let one = Fx::new("json-one");
    let v = json_of(&one.run(&["--json"]));
    let p = jpane(&v, "wA:p1");
    assert_eq!(
        (&p["server"], &p["key"]),
        (&json!(null), &json!("wA:p1")),
        "1 session なら server は null のはず"
    );
    // 2 session: server は session 名、key は <server>/<pane_id>
    let two = Fx::new("json-two");
    two.write("herdr/herdr.sock", "");
    two.write("herdr/sessions/beta/herdr.sock", "");
    let v = json_of(&two.run(&["--json"]));
    let beta = v["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["server"] == "beta" && p["pane_id"] == "wA:p1")
        .unwrap_or_else(|| panic!("beta の wA:p1 が無い: {v}"));
    assert_eq!(
        beta["key"],
        json!("beta/wA:p1"),
        "key は server/pane_id のはず"
    );
    assert!(
        v["panes"].as_array().unwrap().iter().all(|p| {
            let s = p["server"]
                .as_str()
                .unwrap_or_else(|| panic!("server が null: {p}"));
            p["key"] == json!(format!("{s}/{}", p["pane_id"].as_str().unwrap()))
        }),
        "全 pane が server/pane_id の key を持つはず: {v}"
    );
}

#[test]
fn json_strips_escape_from_label_and_owner() {
    let fx = Fx::new("json-esc");
    let mut panes: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fx.path("herdr/panes.json")).unwrap()).unwrap();
    panes["result"]["panes"].as_array_mut().unwrap().push(pane(
        "wA:p5",
        "working",
        SID2,
        "tA:2",
        "wA",
        "\u{1b}[31mrepo-123",
    ));
    fx.write("herdr/panes.json", &panes.to_string());
    let v = json_of(&fx.run(&["--json"]));
    let p = jpane(&v, "wA:p5");
    assert_eq!(
        p["owner"].as_str().map(|o| o.contains('\u{1b}')),
        Some(false),
        "owner に ESC が無いはず: {p}"
    );
    assert!(
        !p["label"].as_str().unwrap().contains('\u{1b}'),
        "label に ESC が無いはず: {p}"
    );
    assert!(
        !stdout(&fx.run(&["--json"])).contains('\u{1b}'),
        "出力全体に ESC が無いはず"
    );
}

// --json の watchers。state_dir と ps は tempdir の stub に差し替え、実 state・実 ps には触れない
const WATCH_ENV: [&str; 4] = [
    "WATCH_PANE_TURNS_STATE_DIR",
    "WATCH_PR_REVIEWS_STATE_DIR",
    "WATCH_REVIEW_REQUESTS_STATE_DIR",
    "HERDR_TIMELINE_PS_BIN",
];

/// stub ps。-p は PID ごとの応答、-axo は ps-all の中身を返す (SELF は herdr-timeline の PID に置換)
fn watch_fx(name: &str) -> (Fx, Vec<(&'static str, String)>) {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new(name);
    fx.write("ps/ps.sh", "#!/bin/sh\nd=$(dirname \"$0\")\ncase \"$1\" in\n  -o) [ -f \"$d/ps-pid-$4\" ] && cat \"$d/ps-pid-$4\" ;;\n  -axo) sed \"s/SELF/$PPID/\" \"$d/ps-all\" ;;\nesac\n");
    fs::set_permissions(fx.path("ps/ps.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    fx.write("ps/ps-all", "");
    let env = vec![
        (WATCH_ENV[0], fx.path("st/turns").display().to_string()),
        (WATCH_ENV[1], fx.path("st/pr").display().to_string()),
        (WATCH_ENV[2], fx.path("st/req").display().to_string()),
        (WATCH_ENV[3], fx.path("ps/ps.sh").display().to_string()),
    ];
    (fx, env)
}

fn watchers_of(fx: &Fx, env: &[(&str, String)]) -> Vec<serde_json::Value> {
    let v = json_of(&fx.run_env(&["--json"], true, env));
    v["watchers"].as_array().unwrap().clone()
}

#[test]
fn watchers_are_four_in_fixed_order_with_env_state_dir() {
    let (fx, env) = watch_fx("w-order");
    let w = watchers_of(&fx, &env);
    let names: Vec<_> = w.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "watch-pane-turns",
            "watch-my-pr-reviews",
            "watch-other-pr-reviews",
            "watch-merged-prs"
        ]
    );
    assert_eq!(w[0]["state_dir"], env[0].1);
    assert_eq!(w[1]["state_dir"], env[1].1);
    assert_eq!(w[2]["state_dir"], env[2].1);
    assert_eq!(
        w[3]["state_dir"], env[1].1,
        "merged は my-pr と同じ state_dir"
    );
    assert!(w
        .iter()
        .all(|x| x["running"] == false && x["pane"].is_null()));
}

#[test]
fn watchers_running_follows_lock_pid_and_ps() {
    let (fx, env) = watch_fx("w-lock");
    fx.write("st/pr/resident.lock/pid", "4242\n");
    fx.write(
        "ps/ps-pid-4242",
        "S+ /bin/bash /x/watch-my-pr-reviews.sh --loop\n",
    );
    let w = watchers_of(&fx, &env);
    assert_eq!(w[1]["running"], true);
    assert_eq!(
        w[3]["running"], false,
        "merged の lock は別 (pid ファイル無し)"
    );
    for line in [
        "S bash -e /x/watch-my-pr-reviews.sh\n",
        "S bash -o pipefail /x/watch-my-pr-reviews.sh\n",
    ] {
        fx.write("ps/ps-pid-4242", line);
        assert_eq!(watchers_of(&fx, &env)[1]["running"], true, "{line:?}");
    }
    for line in [
        "S vim /x/watch-my-pr-reviews.sh\n",
        "S bash /x/watch-my-pr-reviews.sh.bak\n",
        "S bash /x/other.sh /x/watch-my-pr-reviews.sh\n",
        "S bash -o pipefail /x/other.sh /x/watch-my-pr-reviews.sh\n",
        "S bash -c /x/watch-my-pr-reviews.sh\n",
        "S bash -ec /x/watch-my-pr-reviews.sh\n",
        "S bash -s -- /x/watch-my-pr-reviews.sh\n",
        "S bash -es /x/watch-my-pr-reviews.sh\n",
        // ps では空白入りのパスと、空白で区切った複数の引数を区別できない
        "S bash /x/with space/watch-my-pr-reviews.sh\n",
        "T bash /x/watch-my-pr-reviews.sh\n",
        "Z /x/watch-my-pr-reviews.sh\n",
    ] {
        fx.write("ps/ps-pid-4242", line);
        assert_eq!(watchers_of(&fx, &env)[1]["running"], false, "{line:?}");
    }
    // PID が終了済みなら ps の問い合わせは失敗する
    fx.write("ps/ps-pid-4242", "S /x/watch-my-pr-reviews.sh\n");
    fx.write("st/pr/resident.lock/pid", "4243\n");
    assert_eq!(watchers_of(&fx, &env)[1]["running"], false);
    fx.write("st/pr/resident.lock/pid", "abc\n");
    fx.write("ps/ps-pid-abc", "S /x/watch-my-pr-reviews.sh\n");
    assert_eq!(watchers_of(&fx, &env)[1]["running"], false);
    // merged は merged-resident.lock を見る
    fx.write("st/pr/merged-resident.lock/pid", "7\n");
    fx.write("ps/ps-pid-7", "S /x/watch-merged-prs.sh\n");
    assert_eq!(watchers_of(&fx, &env)[3]["running"], true);
}

#[test]
fn watchers_pane_turns_uses_full_ps_listing_without_itself() {
    let (fx, env) = watch_fx("w-turns");
    fx.write(
        "ps/ps-all",
        concat!(
            "SELF S /x/watch-pane-turns.sh\n",
            "  1 S vim /x/watch-pane-turns.sh\n",
            "  2 S /x/watch-pane-turns.sh.bak\n",
            "  3 S bash /x/other.sh /x/watch-pane-turns.sh\n",
            "  4 T /x/watch-pane-turns.sh\n",
        ),
    );
    assert_eq!(
        watchers_of(&fx, &env)[0]["running"],
        false,
        "自分自身と、対象の watcher が稼働していない行は数えない"
    );
    fx.write("ps/ps-all", " 22 S /x/watch-pane-turns.sh\n");
    assert_eq!(watchers_of(&fx, &env)[0]["running"], true);
    // 自分でない pid の行は、path に herdr-timeline を含んでも数える
    fx.write(
        "ps/ps-all",
        "SELF S herdr-timeline --json\n33 S bash -- /x/herdr-timeline/scripts/watch-pane-turns.sh\n",
    );
    assert_eq!(watchers_of(&fx, &env)[0]["running"], true);
    fx.write("ps/ps-all", " 22 S bash -eux /x/watch-pane-turns.sh\n");
    assert_eq!(watchers_of(&fx, &env)[0]["running"], true);
}

#[test]
fn watchers_last_activity_is_newest_entry_mtime_ignoring_lock_dir() {
    let (fx, env) = watch_fx("w-mtime");
    assert!(
        watchers_of(&fx, &env)[1]["last_activity"].is_null(),
        "state_dir 無しは null"
    );
    fx.write("st/pr/a.json", "{}");
    fx.write("st/pr/resident.lock/pid", "1");
    let t = |p: &str, secs: u64| {
        let f = fs::File::options().write(true).open(fx.path(p)).unwrap();
        f.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    };
    t("st/pr/a.json", 1_000_000);
    // lock ディレクトリ配下の更新は無視される (直下の mtime だけを見る)
    let w = watchers_of(&fx, &env);
    assert_eq!(w[1]["last_activity"], 1_000_000);
    assert!(w[0]["last_activity"].is_null());
}

#[test]
fn watchers_pane_is_exact_label_match_or_null() {
    let (fx, env) = watch_fx("w-pane");
    let mut panes: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fx.path("herdr/panes.json")).unwrap()).unwrap();
    let list = panes["result"]["panes"].as_array_mut().unwrap();
    list.push(pane(
        "wA:p8",
        "idle",
        SID2,
        "tA:2",
        "wA",
        "watch-my-pr-reviews",
    ));
    list.push(pane(
        "wA:p9",
        "idle",
        SID2,
        "tA:2",
        "wA",
        "watch-my-pr-reviews-2",
    ));
    fx.write("herdr/panes.json", &panes.to_string());
    let w = watchers_of(&fx, &env);
    assert_eq!(w[1]["pane"], "wA:p8");
    assert!(w[2]["pane"].is_null());
}

#[test]
fn watchers_empty_env_state_dir_falls_back_to_default() {
    let (fx, mut env) = watch_fx("w-empty");
    env[1].1 = String::new();
    let home = fx.path("home").display().to_string();
    env.push(("HOME", home.clone()));
    let w = watchers_of(&fx, &env);
    assert_eq!(
        w[1]["state_dir"],
        format!("{home}/.agents/state/watch-pr-reviews")
    );
}

#[test]
fn watchers_last_activity_sees_one_level_below_subdirs() {
    let (fx, env) = watch_fx("w-deep");
    fx.write("st/turns/advance-notice/x.json", "{}");
    fx.write("st/turns/advance-notice/deeper/y.json", "{}");
    let set = |p: &str, secs: u64| {
        let f = fs::File::open(fx.path(p)).unwrap();
        f.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    };
    set("st/turns/advance-notice/x.json", 3_000_000);
    set("st/turns/advance-notice/deeper/y.json", 9_000_000);
    set("st/turns/advance-notice/deeper", 1);
    set("st/turns/advance-notice", 1);
    // 2 階層より下 (deeper/y.json) は見ない。1 段下の x.json の mtime が出る
    assert_eq!(watchers_of(&fx, &env)[0]["last_activity"], 3_000_000);
}

#[test]
fn codex_timeline_uses_only_the_reported_session_and_its_user_items() {
    let fx = Fx::new("codex-timeline");
    let mut panes: serde_json::Value =
        serde_json::from_slice(&fs::read(fx.path("herdr/panes.json")).unwrap()).unwrap();
    let row = &mut panes["result"]["panes"][1];
    row["agent"] = json!("codex");
    let session = json!({"kind": "id", "agent": "codex", "source": "herdr:codex", "value": SID2});
    row["agent_session"] = session.clone();
    fx.write("herdr/panes.json", &panes.to_string());
    fx.transcript(SID2, &[user("2026-11-21T06:00:01Z", "WRONG_CLAUDE")]);
    let rollout =
        json!({"timestamp":"2026-11-21T06:00:00Z", "type":"session_meta", "payload":{"id":SID2}});
    let instruction = |id: &str, text: &str| {
        json!({
        "timestamp":"2026-11-21T06:00:01Z", "type":"event_msg", "payload":{
            "type":"item_completed", "thread_id":id, "item":{
                "type":"UserMessage","content":[{"type":"text","text":text}]}}})
    };
    let user_replica = json!({"timestamp":"2026-11-21T06:00:01Z", "type":"response_item", "payload":{
        "type":"message", "role":"user", "content":[{"type":"input_text","text":"WRONG_MODEL_COPY"}]}});
    let good = format!(
        "{rollout}\n{}\n{}\n{user_replica}\n",
        instruction(SID2, "CODEX_INSTRUCTION"),
        instruction(SID1, "WRONG_THREAD"),
    );
    let file = format!("codex/sessions/2026/11/21/rollout-2026-11-21T06-00-00-{SID2}.jsonl");
    fx.write(&file, &good);
    fx.msg(
        "runs/r1/inbox/new/1-1795240802000000-info-wA:p2-1.msg",
        "wA:p2",
        Some(&fx.sock()),
        "info",
        "2026-11-21T06:00:02Z",
        "CODEX_REPORT",
    );

    let read =
        || String::from_utf8(fx.run(&["--timeline", "wA:p2", "--width", "150"]).stdout).unwrap();
    let out = read();
    assert_eq!(out.matches("CODEX_INSTRUCTION").count(), 1, "{out}");
    assert!(
        out.contains("CODEX_REPORT") && !out.contains("WRONG_"),
        "{out}"
    );
    let out = fx.run(&["--json"]);
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let p = doc["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pane_id"] == "wA:p2")
        .unwrap();
    assert!(p["working_secs"].is_number());
    assert!(p["queue"].is_null(), "Codex のキュー件数は未観測");
    assert_eq!(p["inbox"]["count"], 1);

    panes["result"]["panes"][1]["agent_session"] = json!(null);
    fx.write("herdr/panes.json", &panes.to_string());
    assert!(read().contains("transcript が見つからない"));
    panes["result"]["panes"][1]["agent_session"] = session.clone();
    panes["result"]["panes"][1]["agent_session"]["source"] = json!("other");
    fx.write("herdr/panes.json", &panes.to_string());
    assert!(read().contains("transcript が見つからない"));
    panes["result"]["panes"][1]["agent_session"] = session;
    fx.write("herdr/panes.json", &panes.to_string());
    fx.write(&file, &good.replacen(SID2, SID1, 1));
    assert!(read().contains("transcript が見つからない"));
    fx.write(&file, &good);
    fx.write(
        &format!("codex/sessions/2026/11/22/rollout-2026-11-22T06-00-00-{SID2}.jsonl"),
        &good,
    );
    assert!(read().contains("transcript が見つからない"));

    use std::os::unix::fs::PermissionsExt;
    let hidden_day = fx.path("codex/sessions/2026/11/22");
    fs::set_permissions(&hidden_day, fs::Permissions::from_mode(0o000)).unwrap();
    let unreadable = fs::read_dir(&hidden_day).is_err();
    let out = read();
    fs::set_permissions(&hidden_day, fs::Permissions::from_mode(0o700)).unwrap();
    if unreadable {
        assert!(out.contains("transcript が見つからない"), "{out}");
    }
}
