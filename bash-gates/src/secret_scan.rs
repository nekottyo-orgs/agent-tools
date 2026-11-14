// 目的: git の unified diff を stdin で受け、commit される追加行から実環境の識別子と
//       credential を検出して JSON で返す (`bash-gates secret-scan` subcommand)。
//       判定と deny は呼び出し側 (シェル) の責務で、ここは「何がどこで見つかったか」だけを返す。
//       hooks/_shared/secret-scan.py の Rust 移植で、出力は JSON として同値になるようにする。
//
// 安全設計 (why):
//   - 誤検知は除外リストでなく構造で潰す。撃つ gate は無視される gate になるため。
//     そのための判定はルール表 (secret_rules.rs) 側に置く
//   - 検査するのは追加行のみ。既に commit 済みの識別子を毎回撃つと、対象 repo の
//     commit が恒久的に deny されて初日で破綻する
//   - 差分検証で Python 実装と一致させるため、Python 実装の癖もそのまま再現する
//     (行内容が `++ ` で始まる追加行を `+++ ` ヘッダと誤認する、など)
//   - 正規表現の先読み・後読みが探索上限で Err になったときは、見落とさないよう検出側に倒す
//     (値は `…`)。fancy-regex は Python の re と違い、上限超過を結果でなく Err で返す
//   - policy の形が想定と違うときは黙って通さず exit 2 にする。素通りは fail-open になる
//   - 値は credential 系 class では先頭 6 文字だけ残して報告する (transcript とログに秘密の全体を残さない)
//
// 使い方: git diff --cached --unified=0 | bash-gates secret-scan --policy <path> --repo owner/name
//
// 終了コード: 0 = 走査完了 (検出の有無は stdout の JSON)。2 = 引数不正・policy 読み込み失敗 (呼び出し側は fail-close で扱う)

use crate::secret_rules;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{Read, Write};

const PRAGMA: &str = "secret-scan-gate:allow";
const MASKED_CLASSES: [&str; 3] = ["credential", "cloud-credential", "db-credential"];

/// policy の `entropy` 表。`_` で始まる key はコメント用なので読まない。
pub struct Entropy(HashMap<String, f64>);

impl Entropy {
    pub(crate) fn from_policy(v: Option<&Value>) -> Result<Self, String> {
        let mut map = HashMap::new();
        if let Some(v) = v {
            let obj = v.as_object().ok_or("entropy must be an object")?;
            for (k, v) in obj.iter().filter(|(k, _)| !k.starts_with('_')) {
                let n = v.as_f64().ok_or_else(|| format!("entropy.{k} must be a number"))?;
                map.insert(k.clone(), n);
            }
        }
        Ok(Entropy(map))
    }

    pub fn get(&self, key: &str, default: f64) -> f64 {
        self.0.get(key).copied().unwrap_or(default)
    }
}

struct Exempt {
    paths: Vec<String>,
    classes: Vec<String>,
}

fn str_list(entry: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(v) = entry.get(key) else { return Ok(vec![]) };
    let arr = v.as_array().ok_or_else(|| format!("exempt[].{key} must be an array"))?;
    arr.iter()
        .map(|s| s.as_str().map(String::from).ok_or_else(|| format!("exempt[].{key} must be strings")))
        .collect()
}

fn parse_exempts(conf: &Map<String, Value>) -> Result<Vec<Exempt>, String> {
    let Some(v) = conf.get("exempt") else { return Ok(vec![]) };
    let arr = v.as_array().ok_or("exempt must be an array")?;
    arr.iter()
        .map(|e| {
            if !e.is_object() {
                return Err("exempt[] must be an object".to_string());
            }
            Ok(Exempt { paths: str_list(e, "paths")?, classes: str_list(e, "classes")? })
        })
        .collect()
}

enum Tok {
    Star,
    Any,
    Lit(char),
    Set(bool, Vec<(char, char)>),
}

// Python の fnmatch.fnmatch (POSIX) と同じ: `*` は `/` も越える、`?`、`[seq]` `[!seq]`。
// std に glob が無く、外部 crate の glob は `*` が `/` を越えないなど意味が違うので自前で持つ
fn compile_glob(pat: &str) -> Vec<Tok> {
    let p: Vec<char> = pat.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let c = p[i];
        i += 1;
        match c {
            '*' => {
                if !matches!(toks.last(), Some(Tok::Star)) {
                    toks.push(Tok::Star);
                }
            }
            '?' => toks.push(Tok::Any),
            '[' => {
                let mut j = i;
                if p.get(j) == Some(&'!') {
                    j += 1;
                }
                if p.get(j) == Some(&']') {
                    j += 1;
                }
                while j < p.len() && p[j] != ']' {
                    j += 1;
                }
                if j >= p.len() {
                    toks.push(Tok::Lit('['));
                    continue;
                }
                let stuff = &p[i..j];
                i = j + 1;
                let (neg, body) = match stuff.first() {
                    Some('!') => (true, &stuff[1..]),
                    _ => (false, stuff),
                };
                let mut ranges = Vec::new();
                let mut k = 0;
                while k < body.len() {
                    if k + 2 < body.len() && body[k + 1] == '-' {
                        ranges.push((body[k], body[k + 2]));
                        k += 3;
                    } else {
                        ranges.push((body[k], body[k]));
                        k += 1;
                    }
                }
                toks.push(Tok::Set(neg, ranges));
            }
            c => toks.push(Tok::Lit(c)),
        }
    }
    toks
}

fn tok_matches(t: &Tok, c: char) -> bool {
    match t {
        Tok::Any => true,
        Tok::Lit(l) => *l == c,
        Tok::Set(neg, ranges) => ranges.iter().any(|&(lo, hi)| lo <= c && c <= hi) != *neg,
        Tok::Star => false,
    }
}

fn fnmatch(pat: &str, s: &str) -> bool {
    let toks = compile_glob(pat);
    let s: Vec<char> = s.chars().collect();
    let (mut ti, mut si) = (0, 0);
    // 直近の `*` の位置と、それが飲み込んだ分
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        match toks.get(ti) {
            Some(Tok::Star) => {
                star = Some((ti, si));
                ti += 1;
                continue;
            }
            Some(t) if tok_matches(t, s[si]) => {
                ti += 1;
                si += 1;
                continue;
            }
            _ => {}
        }
        let Some((st, ss)) = star else { return false };
        ti = st + 1;
        si = ss + 1;
        star = Some((st, ss + 1));
    }
    toks[ti..].iter().all(|t| matches!(t, Tok::Star))
}

fn is_exempt(exempts: &[Exempt], path: &str, cls: &str, rule: &str) -> bool {
    exempts.iter().any(|e| {
        e.paths.iter().any(|p| fnmatch(p, path))
            && e.classes.iter().any(|spec| {
                if spec.contains('/') {
                    spec.split_once('/').is_some_and(|(c, r)| c == cls && r == rule)
                } else {
                    fnmatch(spec, cls)
                }
            })
    })
}

fn mask(cls: &str, value: &str) -> String {
    if !MASKED_CLASSES.contains(&cls) {
        return value.to_string();
    }
    match value.char_indices().nth(6) {
        Some((i, _)) => format!("{}…", &value[..i]),
        None => "…".to_string(),
    }
}

// 和は文字の初出順に取る。浮動小数の和は順序で丸めが変わり、Python 実装 (dict の挿入順) と
// 閾値付近でずれると判定が割れるため
pub fn shannon_entropy(value: &str) -> f64 {
    if value.is_empty() {
        return 0.0;
    }
    let mut index: HashMap<char, usize> = HashMap::new();
    let mut counts: Vec<usize> = Vec::new();
    for ch in value.chars() {
        let i = *index.entry(ch).or_insert_with(|| {
            counts.push(0);
            counts.len() - 1
        });
        counts[i] += 1;
    }
    let len = value.chars().count() as f64;
    let sum = counts.iter().fold(0.0_f64, |acc, &c| {
        let p = c as f64 / len;
        acc + p * p.log2()
    });
    -sum
}

// Python の str.splitlines() と同じ区切り。`\r\n` は 1 つの区切りで、末尾の空行は作らない
fn splitlines(s: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if !matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}') {
            continue;
        }
        lines.push(&s[start..i]);
        start = i + c.len_utf8();
        if c == '\r' && it.peek().is_some_and(|&(_, n)| n == '\n') {
            it.next();
            start += 1;
        }
    }
    if start < s.len() {
        lines.push(&s[start..]);
    }
    lines
}

// Python の str.strip() は `\x1c`-`\x1f` も空白とみなすが、Rust の trim は含まない
fn py_strip(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c))
}

// `@@ -a,b +c,d @@` の `+` の直後の数字列。Python の `\d` は Unicode の数字も取るが、
// git が出す hunk header は ASCII だけなので ASCII に限る
fn new_start(header: &str) -> u64 {
    header
        .match_indices('+')
        .find_map(|(i, _)| {
            let digits: String = header[i + 1..].chars().take_while(char::is_ascii_digit).collect();
            (!digits.is_empty()).then(|| digits.parse().unwrap_or(u64::MAX))
        })
        .unwrap_or(0)
}

/// unified diff から (パス, 行番号, 行内容) を追加行だけ取り出す。
fn added_lines(diff: &str) -> Vec<(String, u64, &str)> {
    let mut out = Vec::new();
    let mut path: Option<String> = None;
    let mut lineno: u64 = 0;
    for raw in splitlines(diff) {
        if let Some(rest) = raw.strip_prefix("+++ ") {
            let target = py_strip(rest);
            // `+++ b/path` の b/ を剥がす。/dev/null は削除なので追跡しない
            path = (target != "/dev/null").then(|| {
                target.strip_prefix("a/").or_else(|| target.strip_prefix("b/")).unwrap_or(target).to_string()
            });
        } else if raw.starts_with("@@") {
            lineno = new_start(raw);
        } else if raw.starts_with('+') && !raw.starts_with("+++") {
            if let Some(p) = &path {
                out.push((p.clone(), lineno, &raw[1..]));
            }
            lineno = lineno.saturating_add(1);
        }
    }
    out
}

fn parse_args(args: &[String]) -> Result<(String, String), String> {
    let (mut policy, mut repo) = (None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (a.as_str(), None),
        };
        let slot = match name {
            "--policy" => &mut policy,
            "--repo" => &mut repo,
            _ => return Err(format!("unrecognized argument: {a}")),
        };
        *slot = Some(match inline {
            Some(v) => v,
            None => it.next().cloned().ok_or_else(|| format!("{name} requires a value"))?,
        });
    }
    Ok((policy.ok_or("--policy is required")?, repo.ok_or("--repo is required")?))
}

fn load_policy(path: &str) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if !v.is_object() {
        return Err("policy must be an object".into());
    }
    Ok(v)
}

fn emit(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = serde_json::to_writer(&mut out, v);
    let _ = out.flush();
}

pub fn run(args: Vec<String>) -> i32 {
    let (policy_path, repo) = match parse_args(&args) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("secret-scan: {e}");
            return 2;
        }
    };
    let policy = match load_policy(&policy_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("policy load failed: {e}");
            return 2;
        }
    };
    let repos = match policy.get("repos") {
        None => None,
        Some(r) => match r.as_object() {
            Some(o) => Some(o),
            None => {
                eprintln!("policy load failed: repos must be an object");
                return 2;
            }
        },
    };
    let conf = match repos.and_then(|r| r.get(&repo)) {
        None | Some(Value::Null) => {
            // 対象外 repo。呼び出し側で allow になる
            emit(&json!({"findings": [], "scanned": false}));
            return 0;
        }
        Some(Value::Object(c)) => c,
        Some(_) => {
            eprintln!("policy load failed: repos[{repo}] must be an object");
            return 2;
        }
    };
    let (exempts, entropy) = match (parse_exempts(conf), Entropy::from_policy(policy.get("entropy"))) {
        (Ok(x), Ok(e)) => (x, e),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("policy load failed: {e}");
            return 2;
        }
    };
    let rules = secret_rules::build(&entropy);

    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
    // 不正な UTF-8 を含む diff (バイナリ混じり、他 encoding) でも落とさず走査する
    let diff = String::from_utf8_lossy(&input);

    let mut findings = Vec::new();
    for (path, lineno, content) in added_lines(&diff) {
        if content.contains(PRAGMA) {
            continue;
        }
        for r in &rules {
            for caps in r.re.captures_iter(content) {
                let (value, failed) = match &caps {
                    Ok(c) => match (r.check)(c, content) {
                        Some(v) => (v, false),
                        None => continue,
                    },
                    Err(_) => ("…".to_string(), true),
                };
                if !is_exempt(&exempts, &path, r.class, r.id) {
                    findings.push(json!({
                        "file": path,
                        "line": lineno,
                        "class": r.class,
                        "rule": r.id,
                        "value": mask(r.class, &value),
                    }));
                }
                if failed {
                    break;
                }
            }
        }
    }
    emit(&json!({"findings": findings, "scanned": true}));
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitlines_は_python_と同じ区切りを使う() {
        assert_eq!(splitlines("a\nb\r\nc\rd"), ["a", "b", "c", "d"]);
        assert_eq!(splitlines("a\n"), ["a"]);
        assert_eq!(splitlines("a\n\n"), ["a", ""]);
        assert_eq!(splitlines(""), Vec::<&str>::new());
        assert_eq!(splitlines("a\x0bb\x0cc\x1cd\x1de\x1ef\u{85}g\u{2028}h\u{2029}i"), ["a", "b", "c", "d", "e", "f", "g", "h", "i"]);
        // \x1f は区切りではない
        assert_eq!(splitlines("a\x1fb"), ["a\x1fb"]);
        assert_eq!(splitlines("\r\n\r\n"), ["", ""]);
        assert_eq!(splitlines("\r\r\n"), ["", ""]);
    }

    #[test]
    fn fnmatch_は_posix_の_glob() {
        assert!(fnmatch("handovers/**", "handovers/x/n.md"));
        assert!(fnmatch("*.md", "a/b/c.md"));
        assert!(fnmatch("net-*", "net-ipv4"));
        assert!(!fnmatch("net-*", "email"));
        assert!(fnmatch("[!b]at", "cat"));
        assert!(!fnmatch("[!b]at", "bat"));
        assert!(fnmatch("[a-c]x", "bx"));
        assert!(fnmatch("?.rs", "a.rs"));
        assert!(!fnmatch("?.rs", "ab.rs"));
        assert!(fnmatch("a[", "a["));
        assert!(fnmatch("[]]", "]"));
        assert!(fnmatch("a*b*c", "aXbYbZc"));
        assert!(!fnmatch("a*b", "ab/c"));
    }

    #[test]
    fn is_exempt_は_class_と_rule_の二通りを受ける() {
        let ex = vec![Exempt {
            paths: vec!["docs/*".into()],
            classes: vec!["net-*".into(), "credential/generic".into()],
        }];
        assert!(is_exempt(&ex, "docs/a/b.md", "net-ipv4", "ipv4"));
        assert!(is_exempt(&ex, "docs/a.md", "credential", "generic"));
        assert!(!is_exempt(&ex, "docs/a.md", "credential", "slack"));
        assert!(!is_exempt(&ex, "src/a.md", "net-ipv4", "ipv4"));
    }

    #[test]
    fn mask_は_credential_系だけ値を削る() {
        assert_eq!(mask("credential", "abcdefgh"), "abcdef…");
        assert_eq!(mask("credential", "abcdef"), "…");
        assert_eq!(mask("db-credential", "あいうえおかき"), "あいうえおか…");
        assert_eq!(mask("net-ipv4", "abcdefgh"), "abcdefgh");
    }

    #[test]
    fn shannon_entropy_は_既知の値() {
        assert_eq!(shannon_entropy(""), 0.0);
        assert_eq!(shannon_entropy("aaaa"), 0.0);
        assert_eq!(shannon_entropy("aabb"), 1.0);
    }

    #[test]
    fn added_lines_は_path_と行番号を追う() {
        let diff = "diff --git a/f b/f\n--- a/f\n+++ b/dir/f.txt\n@@ -1,0 +5,2 @@\n+one\n+two\n@@ -9 +20 @@\n+three\n+++ /dev/null\n@@ -1 +1 @@\n+gone\n";
        let got = added_lines(diff);
        assert_eq!(
            got,
            vec![
                ("dir/f.txt".to_string(), 5, "one"),
                ("dir/f.txt".to_string(), 6, "two"),
                ("dir/f.txt".to_string(), 20, "three"),
            ]
        );
    }

    #[test]
    fn added_lines_は_hunk_が無ければ行番号_0_から数える() {
        let got = added_lines("+++ b/f\n+x\n+y\n");
        assert_eq!(got, vec![("f".to_string(), 0, "x"), ("f".to_string(), 1, "y")]);
    }

    #[test]
    fn added_lines_は_python_版の癖を再現する() {
        // 内容が `++ ` で始まる追加行は `+++ ` ヘッダとして扱われ、path が切り替わる
        let got = added_lines("+++ b/f\n+a\n+++ b\n+c\n");
        assert_eq!(got, vec![("f".to_string(), 0, "a"), ("b".to_string(), 1, "c")]);
        // `+++` だけの行は追加行にならず行番号も進まない
        let got = added_lines("+++ b/f\n+++\n+a\n");
        assert_eq!(got, vec![("f".to_string(), 0, "a")]);
    }

    #[test]
    fn added_lines_は_path_が_none_でも行番号を進める() {
        let got = added_lines("+++ /dev/null\n+x\n+++ b/f\n+y\n");
        assert_eq!(got, vec![("f".to_string(), 1, "y")]);
        let got = added_lines("+++ /dev/null\n@@ -0,0 +1 @@\n+x\n+++ b/f\n+y\n");
        assert_eq!(got, vec![("f".to_string(), 2, "y")]);
    }

    #[test]
    fn parse_args_は_等号形も受ける() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_args(&a(&["--policy", "p", "--repo=o/r"])), Ok(("p".into(), "o/r".into())));
        assert!(parse_args(&a(&["--policy", "p"])).is_err());
        assert!(parse_args(&a(&["--policy"])).is_err());
        assert!(parse_args(&a(&["--x", "1"])).is_err());
    }
}
