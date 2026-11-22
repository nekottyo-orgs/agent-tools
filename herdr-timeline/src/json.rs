// --json の出力。画面と同じ gather と Briefs を通し、pane ごとの状態を 1 つの JSON object にする。
// socket・transcript のパス・session_id は出さない。

use crate::core::Pane;
use crate::data::{
    mine, read_msgs, run_herdr, Briefs, Env, InMsg, PaneKind, PaneRow, Snapshot, TranscriptPaths,
};
use crate::text::clean;
use crate::watchers;
use serde_json::{json, Value};
use std::path::Path;

/// 画面の末尾に出ていたら anomaly とみなす文字列。足すときはここに 1 行足す (部分一致、先頭側が優先)
const ANOMALY_PATTERNS: &[&str] = &["Reconnect failed"];

/// anomaly を探す末尾の行数
const ANOMALY_LINES: &str = "20";

/// 画面の文字列のうち、一覧のどれかを含む最初の要素。出力には画面の文字列を混ぜず、この要素だけを使う
fn match_anomaly(screen: &str) -> Option<&'static str> {
    ANOMALY_PATTERNS
        .iter()
        .copied()
        .find(|a| screen.contains(a))
}

/// working / blocked の pane だけ画面の末尾を読む。読めない (pane read の失敗) ときは null で、warnings には積まない
/// (anomaly の有無は画面が読めたときにしか分からず、pane 単位の失敗を warnings に積むと 1 回の --json で数が膨らむため)
fn anomaly(env: &Env, p: &PaneRow) -> Option<&'static str> {
    if p.status != "working" && p.status != "blocked" {
        return None;
    }
    let args = ["pane", "read", p.pane.as_str(), "--lines", ANOMALY_LINES];
    let out = run_herdr(env, &args, Some(Path::new(&p.socket))).ok()?;
    match_anomaly(&String::from_utf8_lossy(&out))
}

/// {"panes": [...], "warnings": [...], "watchers": [...]}。warnings は一覧取得 (pane / workspace / tab) の失敗で panes が欠けることがあるので必要
pub fn document(
    env: &Env,
    snap: &Snapshot,
    now: i64,
    dirs: &TranscriptPaths,
    briefs: &mut Briefs,
) -> Value {
    let msgs = read_msgs(&snap.files, false);
    // pane read は pane ごとに子プロセスを待つので、遅くならないよう pane 数ぶん並列に呼ぶ
    let anomalies: Vec<Option<&'static str>> = std::thread::scope(|s| {
        let hs: Vec<_> = snap
            .panes
            .iter()
            .map(|p| s.spawn(move || anomaly(env, p)))
            .collect();
        hs.into_iter().map(|h| h.join().unwrap_or(None)).collect()
    });
    let panes: Vec<Value> = snap
        .panes
        .iter()
        .zip(anomalies)
        .map(|(p, a)| pane_json(p, &msgs, now, dirs, briefs, a))
        .collect();
    let watchers = watchers::list(&panes);
    json!({"panes": panes, "warnings": snap.warn, "watchers": watchers})
}

fn pane_json(
    p: &PaneRow,
    msgs: &[InMsg],
    now: i64,
    dirs: &TranscriptPaths,
    briefs: &mut Briefs,
    anomaly: Option<&str>,
) -> Value {
    let label = clean(&p.label);
    let track = dirs.find(p).and_then(|t| briefs.scan(&t, p.codex));
    // 経過の起点が transcript から取れたときだけ。観測起点しか無い (approx) ときは null
    let working_secs = track
        .as_ref()
        .and_then(|t| t.last_turn_ts)
        .filter(|_| p.status == "working")
        // now の取得後に transcript を読むので、その間の入力で起点が now を越えうる。負にしない
        .map(|t| (now - t).max(0));
    // transcript が無い、または最初の時刻が取れない pane は帰属を判定できないので null (0 件と区別する)
    // 届いていない入力 (キュー待ち) と、最後に届いた入力の時刻。first_ts が取れない pane は inbox と同じく null
    let queue = track
        .as_ref()
        .filter(|t| !p.codex && t.first_ts.is_some())
        .map(|t| {
            let (pending, oldest_at) = t.queue.pending();
            json!({"pending": pending, "oldest_at": oldest_at, "last_input_at": t.last_input_ts()})
        });
    let inbox = track.and_then(|t| t.first_ts).map(|since| {
        let pane = Pane {
            pane: p.pane.clone(),
            socket: p.socket.clone(),
            since: Some(since),
        };
        let mine = mine(msgs, &pane);
        json!({"count": mine.len(), "last_at": mine.iter().map(|m| m.msg.ts).max()})
    });
    let owner = match &p.kind {
        Some(PaneKind::Resident) => Some(label.clone()).filter(|l| l.starts_with("watch-")),
        Some(PaneKind::Dispatch(r)) => Some(clean(r)),
        None => None,
    };
    json!({
        "server": Some(&p.ws.0).filter(|s| !s.is_empty()),
        "pane_id": p.pane,
        "key": p.tag,
        "label": label,
        "status": p.status,
        "working_secs": working_secs,
        "inbox": inbox,
        "queue": queue,
        "hidden": p.hidden(),
        "hidden_reason": p.hidden().then_some(match p.kind {
            Some(PaneKind::Resident) => "resident",
            _ => "dispatch",
        }),
        "owner": owner,
        "anomaly": anomaly,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 一覧の要素そのものを返す() {
        assert_eq!(
            match_anomaly("x\nReconnect failed (3)\n"),
            Some("Reconnect failed")
        );
    }

    #[test]
    fn 含まなければ_none() {
        assert_eq!(match_anomaly("all good"), None);
    }

    #[test]
    fn working_と_blocked_以外は読まない() {
        // env が不正でも、読みに行かなければ None のまま返る
        let env = Env::from_env(None, false, false);
        for st in ["done", "idle", "unknown"] {
            let row = PaneRow {
                key: String::new(),
                pane: "wA:p1".into(),
                tag: String::new(),
                status: st.into(),
                name: String::new(),
                title: String::new(),
                label: String::new(),
                kind: None,
                sid: None,
                codex: false,
                tab_target: None,
                ws: (String::new(), String::new()),
                socket: String::new(),
            };
            assert_eq!(anomaly(&env, &row), None);
        }
    }
}
