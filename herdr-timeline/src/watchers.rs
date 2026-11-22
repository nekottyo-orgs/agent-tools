// --json の watchers。常駐 watcher 4 本の稼働と state_dir の最終更新を返す。

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
            let dir = match std::env::var(env).ok().filter(|d| !d.is_empty()) {
                Some(d) => d,
                None => match std::env::var("HOME") {
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
    let o = Command::new(ps).args(args).arg("-ww").output().ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn bash_script_arg<'a>(mut args: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    while let Some(arg) = args.next() {
        match arg {
            "--" => return args.next(),
            "-o" | "+o" | "-O" | "+O" => {
                args.next()?;
            }
            // -c や -s の後ろに現れる引数をスクリプトと取り違えないよう、対応するフラグだけを読み飛ばす。
            _ => match arg.strip_prefix(['-', '+']) {
                Some(flags)
                    if !flags.is_empty()
                        && flags
                            .bytes()
                            .all(|flag| b"abBCeEfhHkmpPTuvx".contains(&flag)) => {}
                Some(_) => return None,
                None => return Some(arg),
            },
        }
    }
    None
}

fn watcher_process<'a>(mut fields: impl Iterator<Item = &'a str>, script: &str) -> bool {
    let Some(state) = fields.next() else {
        return false;
    };
    if state.starts_with(['T', 't', 'Z', 'X']) {
        return false;
    }
    // shortcut: ps は引数の境界を残さないため空白を含むパスは対象外。必要になったら argv を直接取得する。
    let basename = |arg: &'a str| arg.rsplit('/').next().unwrap_or(arg);
    match fields.next().map(basename) {
        Some("bash") => bash_script_arg(fields).map(basename) == Some(script),
        command => command == Some(script),
    }
}

fn pid_running(ps: &str, pidfile: &Path, script: &str) -> bool {
    let Ok(s) = std::fs::read_to_string(pidfile) else {
        return false;
    };
    let pid = s.trim();
    !pid.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && ps_out(ps, &["-o", "stat=,command=", "-p", pid]).is_some_and(|o| {
            o.lines()
                .any(|line| watcher_process(line.split_ascii_whitespace(), script))
        })
}

fn any_running(ps: &str, script: &str) -> bool {
    let me = std::process::id().to_string();
    ps_out(ps, &["-axo", "pid=,stat=,command="]).is_some_and(|o| {
        o.lines().any(|line| {
            let mut fields = line.split_ascii_whitespace();
            fields.next() != Some(me.as_str()) && watcher_process(fields, script)
        })
    })
}

/// 直下と、直下のディレクトリの 1 段下のエントリの mtime の最大 (epoch 秒)。lock ディレクトリ (*.lock) は除く。
/// watch-pane-turns は直下が advance-notice/ などのディレクトリで、直下の mtime は中身の書き込みを映さないため 2 階層まで見る
fn last_activity(dir: &str) -> Option<i64> {
    fn mtime(e: &std::fs::DirEntry) -> Option<i64> {
        let d = e
            .metadata()
            .ok()?
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?;
        Some(d.as_secs() as i64)
    }
    let not_lock = |e: &std::fs::DirEntry| !e.file_name().to_string_lossy().ends_with(".lock");
    let mut best = None;
    for e in std::fs::read_dir(dir).ok()?.flatten().filter(not_lock) {
        best = best.max(mtime(&e));
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            for c in std::fs::read_dir(e.path())
                .into_iter()
                .flatten()
                .flatten()
                .filter(not_lock)
            {
                best = best.max(mtime(&c));
            }
        }
    }
    best
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
