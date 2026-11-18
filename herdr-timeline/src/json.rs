// --json の出力。画面と同じ gather と Briefs を通し、pane ごとの状態を 1 つの JSON object にする。
// socket・transcript のパス・session_id は出さない。

use crate::core::Pane;
use crate::data::{find_transcript, mine, read_msgs, Briefs, InMsg, PaneKind, PaneRow, Snapshot};
use crate::text::clean;
use crate::watchers;
use serde_json::{json, Value};
use std::path::PathBuf;

/// {"panes": [...], "warnings": [...], "watchers": [...]}。warnings は一覧取得 (pane / workspace / tab) の失敗で panes が欠けることがあるので必要
pub fn document(snap: &Snapshot, now: i64, dirs: &[PathBuf], briefs: &mut Briefs) -> Value {
    let msgs = read_msgs(&snap.files, false);
    let panes: Vec<Value> = snap
        .panes
        .iter()
        .map(|p| pane_json(p, &msgs, now, dirs, briefs))
        .collect();
    let watchers = watchers::list(&panes);
    json!({"panes": panes, "warnings": snap.warn, "watchers": watchers})
}

fn pane_json(
    p: &PaneRow,
    msgs: &[InMsg],
    now: i64,
    dirs: &[PathBuf],
    briefs: &mut Briefs,
) -> Value {
    let label = clean(&p.label);
    let track = find_transcript(dirs, p.sid.as_deref()).and_then(|t| briefs.scan(&t));
    // 経過の起点が transcript から取れたときだけ。観測起点しか無い (approx) ときは null
    let working_secs = track
        .as_ref()
        .and_then(|t| t.last_turn_ts)
        .filter(|_| p.status == "working")
        // now の取得後に transcript を読むので、その間の入力で起点が now を越えうる。負にしない
        .map(|t| (now - t).max(0));
    // transcript が無い、または最初の時刻が取れない pane は帰属を判定できないので null (0 件と区別する)
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
        "hidden": p.hidden(),
        "hidden_reason": p.hidden().then_some(match p.kind {
            Some(PaneKind::Resident) => "resident",
            _ => "dispatch",
        }),
        "owner": owner,
    })
}
