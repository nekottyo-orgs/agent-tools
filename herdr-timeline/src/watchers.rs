// --json の watchers。常駐 watcher 4 本の稼働と state_dir の最終更新を返す。
// 稼働の判定は resident-lock.sh の resident_proc_is と同じ (pid ファイルを読み、ps の command に script 名を含むか)。

use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::time::UNIX_EPOCH;

/// (name, state_dir の env, 既定の相対パス, lock の名前, script 名)。順序が出力の順序
const WATCHERS: [(&str, &str, &str, Option<&str>, &str); 4] = [
    (
        "watch-pane-turns",
        "WATCH_PANE_TURNS_STATE_DIR",
        "watch-pane-turns",
        None,
        "watch-pane-turns.sh",
    ),
    (
        "watch-my-pr-reviews",
        "WATCH_PR_REVIEWS_STATE_DIR",
        "watch-pr-reviews",
        Some("resident.lock"),
        "watch-my-pr-reviews.sh",
    ),
    (
        "watch-other-pr-reviews",
        "WATCH_REVIEW_REQUESTS_STATE_DIR",
        "watch-review-requests",
        Some("resident.lock"),
        "watch-other-pr-reviews.sh",
    ),
    (
        "watch-merged-prs",
        "WATCH_PR_REVIEWS_STATE_DIR",
        "watch-pr-reviews",
        Some("merged-resident.lock"),
        "watch-merged-prs.sh",
    ),
];

/// panes は document() が作った pane の JSON。label 完全一致の先頭の pane_id を pane に入れる
pub fn list(panes: &[Value]) -> Vec<Value> {
    let ps = std::env::var("HERDR_TIMELINE_PS_BIN").unwrap_or_else(|_| "ps".into());
    WATCHERS
        .iter()
        .map(|&(name, env, def, lock, script)| {
            let dir = match std::env::var(env) {
                Ok(d) => d,
                Err(_) => match std::env::var("HOME") {
                    Ok(h) => format!("{h}/.agents/state/{def}"),
                    Err(_) => format!("~/.agents/state/{def}"),
                },
            };
            let running = match lock {
                Some(l) => pid_running(&ps, &Path::new(&dir).join(l).join("pid"), script),
                None => any_running(&ps, script),
            };
            let pane = panes.iter().find(|p| p["label"] == name).map(|p| p["pane_id"].clone());
            json!({"name": name, "running": running, "pane": pane, "state_dir": dir, "last_activity": last_activity(&dir)})
        })
        .collect()
}

fn ps_out(ps: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(ps).args(args).output().ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn pid_running(ps: &str, pidfile: &Path, script: &str) -> bool {
    let Ok(s) = std::fs::read_to_string(pidfile) else {
        return false;
    };
    let pid = s.trim();
    !pid.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && ps_out(ps, &["-o", "command=", "-p", pid]).is_some_and(|o| o.contains(script))
}

fn any_running(ps: &str, script: &str) -> bool {
    // herdr-timeline 自身の行 (引数に script 名が入りうる) は数えない
    ps_out(ps, &["-axo", "command="]).is_some_and(|o| {
        o.lines()
            .any(|l| l.contains(script) && !l.contains("herdr-timeline"))
    })
}

/// 直下のエントリの mtime の最大 (epoch 秒)。lock ディレクトリ (*.lock) は除く。再帰しない
fn last_activity(dir: &str) -> Option<i64> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().ends_with(".lock"))
        .filter_map(|e| {
            e.metadata()
                .ok()?
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()
        })
        .map(|d| d.as_secs() as i64)
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_が数字でなければ_ps_を呼ばず_false() {
        let d = std::env::temp_dir().join(format!("ht-w-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("pid");
        std::fs::write(&f, "12ab\n").unwrap();
        assert!(!pid_running("/nonexistent-ps", &f, "x.sh"));
        std::fs::write(&f, "").unwrap();
        assert!(!pid_running("/nonexistent-ps", &f, "x.sh"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn state_dir_が無ければ_last_activity_は_none() {
        assert_eq!(last_activity("/nonexistent-state-dir-for-test"), None);
    }

    #[test]
    fn ps_が実行できなければ_false() {
        assert!(!any_running("/nonexistent-ps", "x.sh"));
    }
}
