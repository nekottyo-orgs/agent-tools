// 目的: Bash の PreToolUse gate 群を 1 プロセスで受け、command に各 gate のきっかけ語が
//       あるものだけを並列に起動し、結果を 1 つにまとめて返す。
//
// 安全設計 (why):
//   - 律速は bash と jq の起動。gate を別々に登録すると 1 回 約 60ms 待つ。きっかけ語の無い
//     コマンド (実測 52%) は子を 1 本も起動せずに返す
//   - きっかけ語は manifest (hooks-registration.json) の dispatched から実行時に読む。gate を足したときに
//     ここを直し忘れる経路を作らないため。読む費用は起動費用に比べて小さい
//   - 照合は「生の command」と「\ " ' を消した command」の両方に掛け、どちらかに当たれば起動する。
//     gate ごとに quote の外し方が違うので、片方だけだと上位集合にならない
//   - 迷ったら全員起動: 入力 JSON が壊れている、command が文字列でない
//   - 親自身が子を起動できないとき (manifest が読めない・子が 0 本・一時領域を作れない・子を起動できない・panic) は
//     deny。素通りにすると fail-close の gate (secret の検査など) まで黙って止まる
//   - 子の結果の扱い: exit 2 は stderr を理由とする deny、その他の
//     非 0・壊れた出力・timeout は素通り。deny > ask、理由文は同文を 1 つにまとめる
//   - 子の stdin・stdout・stderr はファイルにする。孫 (jq、lint) がパイプを握ったまま残ると、
//     子を kill しても読み取りが返らないため
//   - バイナリが source より古いときは、stdin を読む前に bash-gates.sh (取り込み側が置く退避用の script) へ exec してビルドし直させる。
//     子を起動した後に非 0 で抜けると、登録の `||` が退避側を走らせて子が二重に動くので、
//     この経路以外で非 0 を返さない
//
// 環境変数 (テスト隔離用):
//   BASH_GATES_DIR       子 hook の置き場 (既定はこのバイナリから見た hooks/)
//   BASH_GATES_MANIFEST  manifest (hooks-registration.json) のパス
//   BASH_GATES_FRESH     設定されていれば source との新旧比較を飛ばす (bash-gates.sh がビルド直後に 1 を付ける)
//
// 終了コード: 0。判定は stdout の JSON で返す。stale で bash-gates.sh へ exec したときは、その終了コード。

use serde::Deserialize;
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

const PARENT: &str = "bash-gates.sh";

#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    dispatched: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    command: String,
    via: String,
    #[serde(default = "default_timeout")]
    timeout: u64,
    #[serde(default)]
    words: Vec<String>,
    #[serde(default)]
    substrings: Vec<String>,
}

fn default_timeout() -> u64 {
    10
}

#[derive(Deserialize)]
struct Input {
    tool_input: Option<ToolInput>,
}

#[derive(Deserialize)]
struct ToolInput {
    command: Option<Value>,
}

fn main() {
    let hooks = hooks_dir();
    if std::env::var_os("BASH_GATES_FRESH").is_none() && is_stale(&hooks) {
        // exec が失敗したら古いバイナリのまま続ける (止めるより古い判定で通す方が害が小さい)
        let _ = Command::new(hooks.join(PARENT)).exec();
    }
    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
    // 子を起動した後に panic で非 0 にすると退避側が子を二重に走らせるので、何があっても 0 で抜ける
    let out = std::panic::catch_unwind(|| run(&hooks, &input))
        .unwrap_or_else(|_| deny("bash-gates: 内部エラー (panic) で gate の結果を合成できなかった。stderr を確認すること".into()));
    if let Some(out) = out {
        let _ = std::io::stdout().write_all(out.as_bytes());
    }
}

fn hooks_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("BASH_GATES_DIR") {
        return PathBuf::from(d);
    }
    // hooks/bash-gates/target/release/bash-gates から 4 つ上
    std::env::current_exe()
        .ok()
        .and_then(|p| p.ancestors().nth(4).map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn is_stale(hooks: &Path) -> bool {
    let Some(bin) = std::env::current_exe().ok().and_then(|p| mtime(&p)) else {
        return false;
    };
    let src = hooks.join("bash-gates");
    ["Cargo.toml", "Cargo.lock", "src/main.rs"]
        .iter()
        .any(|f| mtime(&src.join(f)).is_some_and(|t| t > bin))
}

fn manifest_path(hooks: &Path) -> PathBuf {
    std::env::var_os("BASH_GATES_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| hooks.join("../hooks-registration.json"))
}

fn is_wordish(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-')
}

// w がコマンド語として現れるか。前は行頭か語を作らない文字、後ろは行末か語を作らない文字
fn has_word(s: &[u8], w: &[u8]) -> bool {
    if w.is_empty() || s.len() < w.len() {
        return false;
    }
    (0..=s.len() - w.len()).any(|i| {
        &s[i..i + w.len()] == w
            && (i == 0 || !is_wordish(s[i - 1]))
            && (i + w.len() == s.len() || !is_wordish(s[i + w.len()]))
    })
}

fn triggered(e: &Entry, texts: &[&str]) -> bool {
    texts.iter().any(|t| {
        e.words.iter().any(|w| has_word(t.as_bytes(), w.as_bytes()))
            || e.substrings.iter().any(|s| t.contains(s.as_str()))
    })
}

fn select(entries: Vec<Entry>, input: &[u8]) -> Vec<Entry> {
    let command = serde_json::from_slice::<Input>(input)
        .ok()
        .and_then(|i| i.tool_input)
        .and_then(|t| t.command);
    let Some(Value::String(raw)) = command else {
        return entries;
    };
    let stripped: String = raw.chars().filter(|c| !matches!(c, '\\' | '"' | '\'')).collect();
    let texts = [raw.as_str(), stripped.as_str()];
    entries.into_iter().filter(|e| triggered(e, &texts)).collect()
}

struct Outcome {
    name: String,
    decision: Option<String>,
    reason: String,
    ctx: Option<String>,
    sys: Option<String>,
}

// 親が子を起動できないときの返り値。黙って通すと secret の検査等が fail-open になる
fn deny(reason: String) -> Option<String> {
    Some(json!({"hookSpecificOutput": {"hookEventName": "PreToolUse",
        "permissionDecision": "deny", "permissionDecisionReason": reason}}).to_string())
}

fn run(hooks: &Path, input: &[u8]) -> Option<String> {
    let path = manifest_path(hooks);
    let Some(manifest) = fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Manifest>(&b).ok())
    else {
        return deny(format!("bash-gates: {} を読めず、Bash の gate を起動できない。ファイルを直してから再実行すること", path.display()));
    };
    let entries: Vec<Entry> = manifest
        .dispatched
        .into_iter()
        .filter(|e| e.via == PARENT)
        .collect();
    if entries.is_empty() {
        return deny(format!("bash-gates: {} の dispatched に via={PARENT} の子が無い。manifest を直してから再実行すること", path.display()));
    }
    let picked = select(entries, input);
    if picked.is_empty() {
        return None;
    }

    // stdin には command 全文 (secret を含みうる) が入るので本人だけが読める 0700 で作る
    let tmp = std::env::temp_dir().join(format!(
        "bash-gates.{}.{}",
        std::process::id(),
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos()
    ));
    let stdin_path = tmp.join("in");
    let ready = std::os::unix::fs::DirBuilderExt::mode(&mut fs::DirBuilder::new(), 0o700).create(&tmp).is_ok()
        && fs::write(&stdin_path, input).is_ok();
    if !ready {
        let _ = fs::remove_dir_all(&tmp);
        return deny(format!("bash-gates: 一時領域 {} を作れず、Bash の gate を起動できない。TMPDIR と空き容量を確認すること", tmp.display()));
    }
    let outcomes = run_children(hooks, &tmp, &stdin_path, &picked);
    let _ = fs::remove_dir_all(&tmp);
    merge(outcomes)
}

fn run_children(hooks: &Path, tmp: &Path, stdin_path: &Path, picked: &[Entry]) -> Vec<Option<Outcome>> {
    let (tx, rx) = mpsc::channel();
    let start = Instant::now();
    let mut pending = Vec::new();
    let mut unspawned = vec![false; picked.len()];
    for (i, e) in picked.iter().enumerate() {
        let spawned = (|| {
            Command::new(hooks.join(&e.command))
                .stdin(File::open(&stdin_path).ok()?)
                .stdout(File::create(tmp.join(format!("{i}.out"))).ok()?)
                .stderr(File::create(tmp.join(format!("{i}.err"))).ok()?)
                .spawn()
                .ok()
        })();
        let Some(mut child) = spawned else {
            unspawned[i] = true;
            continue;
        };
        let pid = child.id() as libc::pid_t;
        let tx = tx.clone();
        std::thread::spawn(move || {
            let code = child.wait().ok().and_then(|s| s.code());
            let _ = tx.send((i, code));
        });
        pending.push((i, pid, start + Duration::from_secs(e.timeout)));
    }
    drop(tx);

    let mut codes: Vec<Option<i32>> = vec![None; picked.len()];
    while !pending.is_empty() {
        let next = pending.iter().map(|p| p.2).min().unwrap_or(start);
        match rx.recv_timeout(next.saturating_duration_since(Instant::now())) {
            Ok((i, code)) => {
                codes[i] = code;
                pending.retain(|p| p.0 != i);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                pending.retain(|&(_, pid, deadline)| {
                    if deadline <= now {
                        // timeout した子は素通り扱い。待ち受けスレッドは kill 後の wait で抜ける
                        unsafe { libc::kill(pid, libc::SIGKILL) };
                        return false;
                    }
                    true
                });
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    picked
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let name = e.command.trim_end_matches(".sh").to_string();
            // 子の非 0・timeout は素通りが契約だが、起動すらできないのは親の故障なので deny
            if unspawned[i] {
                return Some(Outcome {
                    reason: format!("bash-gates: {} を起動できない。ファイルの有無と実行権を確認すること", hooks.join(&e.command).display()),
                    name,
                    decision: Some("deny".into()),
                    ctx: None,
                    sys: None,
                });
            }
            match codes[i]? {
                2 => Some(Outcome {
                    name,
                    decision: Some("deny".into()),
                    reason: fs::read_to_string(tmp.join(format!("{i}.err"))).unwrap_or_default(),
                    ctx: None,
                    sys: None,
                }),
                0 => {
                    let v: Value = serde_json::from_slice(&fs::read(tmp.join(format!("{i}.out"))).ok()?).ok()?;
                    let h = v.get("hookSpecificOutput");
                    let s = |o: Option<&Value>, k: &str| o.and_then(|o| o.get(k)).and_then(Value::as_str).map(String::from);
                    Some(Outcome {
                        name,
                        decision: s(h, "permissionDecision"),
                        reason: s(h, "permissionDecisionReason").unwrap_or_default(),
                        ctx: s(h, "additionalContext"),
                        sys: s(Some(&v), "systemMessage"),
                    })
                }
                _ => None,
            }
        })
        .collect()
}

// 同じ理由文は 1 つにまとめ、複数残れば子の名前を見出しにして連結する
fn joined(xs: &[&Outcome]) -> String {
    let mut uniq: Vec<&Outcome> = Vec::new();
    for x in xs {
        if !uniq.iter().any(|u| u.reason == x.reason) {
            uniq.push(x);
        }
    }
    if uniq.len() == 1 {
        return uniq[0].reason.clone();
    }
    uniq.iter()
        .map(|u| format!("[{}]\n{}", u.name, u.reason))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn merge(outcomes: Vec<Option<Outcome>>) -> Option<String> {
    let rs: Vec<Outcome> = outcomes.into_iter().flatten().collect();
    let pick = |d: &str| rs.iter().filter(|r| r.decision.as_deref() == Some(d)).collect::<Vec<_>>();
    let (deny, ask) = (pick("deny"), pick("ask"));
    let ctx = rs.iter().filter_map(|r| r.ctx.as_deref()).collect::<Vec<_>>().join("\n\n");
    let sys = rs.iter().filter_map(|r| r.sys.as_deref()).collect::<Vec<_>>().join("\n\n");

    let mut h = json!({"hookEventName": "PreToolUse"});
    if !deny.is_empty() {
        h["permissionDecision"] = "deny".into();
        h["permissionDecisionReason"] = joined(&deny).into();
    } else if !ask.is_empty() {
        h["permissionDecision"] = "ask".into();
        h["permissionDecisionReason"] = joined(&ask).into();
    } else if ctx.is_empty() && sys.is_empty() {
        return None;
    }
    if !ctx.is_empty() {
        h["additionalContext"] = ctx.into();
    }
    let mut out = json!({"hookSpecificOutput": h});
    if !sys.is_empty() {
        out["systemMessage"] = sys.into();
    }
    Some(out.to_string())
}

// lean/BashGates.lean のモデルと関数単位で突き合わせる。表は lean/Vectors.lean が書く (形式はそちらの冒頭)。
// BASH_GATES_EXTRA_VECTORS に同じ形式の表を渡すと、それも比べる (repo に置けない実コマンド由来の表用)
#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(t: &str) -> Vec<u8> {
        let h = t.strip_prefix('=').unwrap_or_else(|| panic!("バイト列の field が = で始まらない: {t}"));
        (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
    }
    fn text(t: &str) -> String {
        String::from_utf8(bytes(t)).unwrap()
    }
    fn opt(t: &str) -> Option<String> {
        (t != "-").then(|| text(t))
    }

    // 1 行を Rust の関数に掛け、Lean の結果と同じなら true
    fn agrees(line: &str) -> bool {
        let mut it = line.split(' ');
        let mut next = || it.next().unwrap_or_else(|| panic!("field が足りない: {line}"));
        match next() {
            "h" => {
                let (s, w) = (bytes(next()), bytes(next()));
                has_word(&s, &w) == (next() == "1")
            }
            "t" => {
                let nw: usize = next().parse().unwrap();
                let words: Vec<String> = (0..nw).map(|_| text(next())).collect();
                let ns: usize = next().parse().unwrap();
                let substrings: Vec<String> = (0..ns).map(|_| text(next())).collect();
                let raw = text(next());
                let (trig, sel) = (next() == "1", next() == "1");
                let entry = || Entry {
                    command: String::new(),
                    via: PARENT.into(),
                    timeout: 10,
                    words: words.clone(),
                    substrings: substrings.clone(),
                };
                let input = serde_json::to_vec(&json!({"tool_input": {"command": raw}})).unwrap();
                triggered(&entry(), &[raw.as_str()]) == trig && (select(vec![entry()], &input).len() == 1) == sel
            }
            "m" => {
                let n: usize = next().parse().unwrap();
                let os: Vec<Option<Outcome>> = (0..n)
                    .map(|_| match next() {
                        "-" => None,
                        _ => Some(Outcome {
                            name: text(next()),
                            decision: opt(next()),
                            reason: text(next()),
                            ctx: opt(next()),
                            sys: opt(next()),
                        }),
                    })
                    .collect();
                // Lean の Out は ctx / sys の空文字で「field 無し」を表す
                let nonempty = |s: String| (!s.is_empty()).then_some(s);
                let want = match next() {
                    "-" => None,
                    _ => Some((opt(next()), opt(next()), nonempty(text(next())), nonempty(text(next())))),
                };
                let got = merge(os).map(|s| {
                    let v: Value = serde_json::from_str(&s).unwrap();
                    let h = &v["hookSpecificOutput"];
                    let f = |x: &Value| x.as_str().map(String::from);
                    (f(&h["permissionDecision"]), f(&h["permissionDecisionReason"]), f(&h["additionalContext"]), f(&v["systemMessage"]))
                });
                got == want
            }
            k => panic!("知らない種類 {k}: {line}"),
        }
    }

    fn check(table: &str) {
        let lines: Vec<&str> = table.lines().collect();
        let bad: Vec<&&str> = lines.iter().filter(|l| !agrees(l)).collect();
        eprintln!("突き合わせ: {} 件中 {} 件一致", lines.len(), lines.len() - bad.len());
        assert!(!lines.is_empty(), "表が空");
        assert!(bad.is_empty(), "Lean と不一致の行 (先頭 5 件):\n{:#?}", &bad[..bad.len().min(5)]);
    }

    #[test]
    fn lean_vectors() {
        check(include_str!("../lean/vectors.txt"));
    }

    #[test]
    fn extra_vectors() {
        if let Some(p) = std::env::var_os("BASH_GATES_EXTRA_VECTORS") {
            check(&fs::read_to_string(p).unwrap());
        }
    }
}
