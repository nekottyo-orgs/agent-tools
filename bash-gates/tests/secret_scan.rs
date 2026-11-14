// `bash-gates secret-scan` (検出エンジン) の integration test。
//
// Python 実装向けに書かれた test のうち、検出エンジンを直接叩く層をここへ移したもの。
// git の staged / commit 形 / bypass / fail-close のような gate (シェル) の責務は対象外。
//
// 値の組み立てについて (why): このファイル自体が commit される追加行として検査される。
// 「止める」側の credential 系は本物の形をソースに連続して残さないよう、prefix と本体を別の
// 断片に割って実行時に連結する。本体は固定 seed の疑似乱数で作る (固定なので閾値付きルールが
// 稀に落ちることはなく、乱数列そのものはソースに現れない)。
//
// SECRET_SCAN_REFERENCE に実行ファイルを渡すと、bash-gates の代わりにそれを
// `secret-scan --policy .. --repo ..` の形で呼ぶ。Python 実装に包んだ wrapper を渡せば、
// このファイルの期待値が Python 実装と一致しているかを確かめられる。

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// 実行の補助
// ---------------------------------------------------------------------------

struct Out {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn reference_mode() -> bool {
    std::env::var_os("SECRET_SCAN_REFERENCE").is_some()
}

fn run_cli(args: &[&str], stdin: &str) -> Out {
    let mut cmd = match std::env::var_os("SECRET_SCAN_REFERENCE") {
        Some(p) => Command::new(p),
        None => Command::new(env!("CARGO_BIN_EXE_bash-gates")),
    };
    let mut child = cmd
        .arg("secret-scan")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("起動に失敗");
    // 引数不正で stdin を読まずに終わっても BrokenPipe で test を落とさない
    let _ = child.stdin.take().expect("stdin").write_all(stdin.as_bytes());
    let o = child.wait_with_output().expect("待機に失敗");
    Out {
        code: o.status.code(),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn scan(policy: &Path, repo: &str, stdin: &str) -> Out {
    run_cli(&["--policy", policy.to_str().unwrap(), "--repo", repo], stdin)
}

/// 一意な一時 dir。外部 crate を足さないので Drop で消す。
struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bash-gates-secret-scan-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("一時 dir を作れない");
        Tmp(dir)
    }

    fn file(&self, name: &str, content: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, content).expect("一時 file を書けない");
        p
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// public 向けの合成 policy。repo 名・path は全て架空。
fn policy_json() -> Value {
    json!({
        "version": 1,
        "repos": {
            "example/repo": {"exempt": []},
            "example/docs": {"exempt": [
                {"paths": ["docs/**"], "classes": ["net-*"]},
                {"paths": ["notes/**"], "classes": ["net-mac/mac"]},
                {"paths": ["extra/**"], "classes": ["credential/generic"]},
                {"paths": ["wild/**"], "classes": ["net-*/mac"]}
            ]}
        },
        "entropy": {
            "generic": 3.0, "aws-secret-key": 4.0, "aws-session-token": 3.5, "datadog": 3.0,
            "splunk-hec": 2.5, "azure-client-secret": 3.5, "db": 2.0
        }
    })
}

fn findings(out: &Out) -> Option<Vec<Value>> {
    let v: Value = serde_json::from_str(&out.stdout).ok()?;
    if out.code != Some(0) || v["scanned"] != json!(true) {
        return None;
    }
    v["findings"].as_array().cloned()
}

/// 検出した rule id のカンマ区切り。検出なしは none、scanner 異常は ERR。
fn rules_of(out: &Out) -> String {
    match findings(out) {
        None => format!("ERR(exit={:?} stderr={})", out.code, out.stderr.trim()),
        Some(f) if f.is_empty() => "none".into(),
        Some(f) => f.iter().map(|x| x["rule"].as_str().unwrap_or("?")).collect::<Vec<_>>().join(","),
    }
}

// ---------------------------------------------------------------------------
// table 駆動 (1 ケース = 説明, 追加行の本文, 期待する rule id の列 または none)
// ---------------------------------------------------------------------------

struct Case {
    desc: String,
    repo: &'static str,
    path: &'static str,
    line: String,
    want: String,
}

fn c(desc: &str, line: impl Into<String>, want: &str) -> Case {
    Case { desc: desc.into(), repo: "example/repo", path: "f.md", line: line.into(), want: want.into() }
}

impl Case {
    fn at(mut self, repo: &'static str, path: &'static str) -> Self {
        self.repo = repo;
        self.path = path;
        self
    }
}

/// stop=true なら期待が検出ありのケース、false なら none のケースだけを走らせる。
/// 1 つ目の失敗で止めず、全ケースを回してから失敗を一覧にして panic する。
fn run(cases: Vec<Case>, stop: bool) {
    let t = Tmp::new();
    let policy = t.file("policy.json", &policy_json().to_string());
    let (mut n, mut ng) = (0, Vec::new());
    for k in cases.iter().filter(|k| (k.want != "none") == stop) {
        n += 1;
        let diff = format!("+++ b/{}\n@@ -0,0 +1 @@\n+{}\n", k.path, k.line);
        let got = rules_of(&scan(&policy, k.repo, &diff));
        if got != k.want {
            ng.push(format!("  NG {}: 期待 {} / 実際 {}", k.desc, k.want, got));
        }
    }
    assert!(n > 0, "走らせるケースが 0 件");
    assert!(ng.is_empty(), "{} 件中 {} 件失敗\n{}", n, ng.len(), ng.join("\n"));
}

macro_rules! stop_pass {
    () => {
        #[test]
        fn stop() {
            run(cases(), true)
        }
        #[test]
        fn pass() {
            run(cases(), false)
        }
    };
}

macro_rules! only_pass {
    () => {
        #[test]
        fn pass() {
            run(cases(), false)
        }
    };
}

// ---------------------------------------------------------------------------
// fixture 値 (実行時に組み立てる)
// ---------------------------------------------------------------------------

const DIGITS: &str = "0123456789";
const HEX: &str = "0123456789abcdef";
const UPNUM: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const ALNUM: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const ALNUM_US: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_";
const URLSAFE: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
const B64: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789/+";
const AZ: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._~-";

fn cat(parts: &[&str]) -> String {
    parts.concat()
}

/// 固定 seed の疑似乱数で alphabet から n 文字を作る。
fn gen(alpha: &str, n: usize, seed: u64) -> String {
    let a = alpha.as_bytes();
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            a[((x >> 33) as usize) % a.len()] as char
        })
        .collect()
}

fn akid(prefix: &str, seed: u64) -> String {
    format!("{prefix}{}", gen(UPNUM, 16, seed))
}

/// 実在しなさそうな 12 桁 (昇順連番でも相異なる数字 4 以下でもない)
fn account() -> String {
    cat(&["5829", "3174", "0615"])
}

fn guid(seed: u64) -> String {
    format!("{}-{}-{}-{}-{}", gen(HEX, 8, seed), gen(HEX, 4, seed + 1), gen(HEX, 4, seed + 2), gen(HEX, 4, seed + 3), gen(HEX, 12, seed + 4))
}

/// 止める側のメール。実在ドメインを避けた合成値
fn mail() -> String {
    cat(&["alice", "@", "fixture-corp", ".io"])
}

fn replace_range(s: &str, from: usize, to: usize, with: &str) -> String {
    format!("{}{}{}", &s[..from], with, &s[to..])
}

// AWS 公式 docs のダミー値 (EXAMPLE を含む)。断片に割り、連続した形をファイルに残さない
fn aws_doc_secret() -> String {
    cat(&["wJalrXUtnFEMI/K7MDENG", "/bPxRfiCY", "EXAMPLEKEY"])
}

fn aws_doc_session() -> String {
    cat(&[
        "AQoEXAMPLEH4aoAH0g", "NCAPyJxz4BlCFFxWNE1", "OPTgk5TthT+FvwqnKwR", "cOIfrRh3c/LTo6UDdyJ", "wOOvEVPvLXCrrrUtdnn", "iCE",
        "XAMPLE/IvU1dYUg2RV", "AJBanLiHb4IgRmpRV3z", "rkuWJOgQs8IZZaIv2BX", "Ia2R4OlgkBN9bkUDNCJ", "iBeb/AXlzBBko7b15fj",
        "rBs2+cTQtpZ3CYWFXG8", "C5zqx37wnOE49mRl/+O", "tkIKGO7fAE",
    ])
}

// ---------------------------------------------------------------------------
// 検出ルールの class 別 (net-* / email / cloud / db / credential)
// ---------------------------------------------------------------------------

mod net_mac {
    use super::*;

    fn cases() -> Vec<Case> {
        vec![
            c("実 MAC (unicast, 予約外)", r#"KNOWN_GATEWAYS="a4:83:e7:11:22:33""#, "mac"),
            c("locally administered な MAC", "gw=02:5a:3c:7f:19:e4", "mac"),
            c("MAC のハイフン区切り (実 MAC)", "gw A4-83-E7-11-22-33 up", "mac"),
            c("arp の先頭ゼロ落ちの実 MAC", "? (192.168.1.1) at a4:83:e7:1:2:3 on en0", "mac"),
            c("RFC 7042 文書用 MAC", r#"KNOWN_GATEWAYS="00:00:5e:00:53:0a 00:00:5e:00:53:0b""#, "none"),
            c("arp の先頭ゼロ落ち + 文書用", "# arp(8) の出力例: ? (192.168.1.1) at 0:0:5e:0:53:a on en0 ifscope [ethernet]", "none"),
            c("multicast (mDNS)", "# arp は先頭ゼロを落とす (例: 1:0:5e:0:0:fb)", "none"),
            c("broadcast", "ff:ff:ff:ff:ff:ff", "none"),
            c("全ゼロ", "mac 00:00:00:00:00:00", "none"),
            c("時刻文字列 (3 グループ)", "21:25:58  sleep disabled: on AC power", "none"),
            c("MAC: 7 グループは撃たない (コロン)", "id a4:83:e7:11:22:33:44", "none"),
            c("MAC: RFC 7042 文書用 (コロン、末尾 ff)", "mac 00:00:5e:00:53:ff", "none"),
            c("MAC: RFC 7042 文書用 (ハイフン)", "doc 00-00-5E-00-53-0A", "none"),
            c("MAC: multicast (ハイフン)", "mdns 01-00-5E-00-00-FB", "none"),
            c("MAC: broadcast (ハイフン)", "ff-ff-ff-ff-ff-ff", "none"),
            c("MAC: 日時の形 (数字だけの 6 グループ)", "build 26-11-15-12-30-45", "none"),
            c("MAC: GUID の途中は撃たない", "id 7c1e4a92-3b5d-4f68-9a0e-b2d4f6a8c0e1", "none"),
            c("MAC: 7 グループ以上は撃たない (前後の - を境界にする)", "id a4-83-e7-11-22-33-44", "none"),
            c("MAC: 接頭辞付き (host-a4-...) は撃たない", "host-a4-83-e7-11-22-33", "none"),
        ]
    }
    stop_pass!();
}

mod net_ipv4 {
    use super::*;

    fn cases() -> Vec<Case> {
        vec![
            c("global IPv4", "resolver=8.8.8.8", "ipv4"),
            c("括弧内の global IPv4", "dns (8.8.8.8) ok", "ipv4"),
            c("単一ホストの /32 は撃つ", "host 8.8.8.8/32", "ipv4"),
            c("RFC 1918 private", "gateway 192.168.1.1", "none"),
            c("RFC 1918 private (10/8)", "gateway 10.0.0.1", "none"),
            c("loopback", "bind 127.0.0.1", "none"),
            c("unspecified", "listen 0.0.0.0", "none"),
            c("link-local", "addr 169.254.1.1", "none"),
            c("共有アドレス空間 (100.64/10)", "addr 100.64.0.1", "none"),
            c("RFC 5737 文書用", "example host 192.0.2.1", "none"),
            c("RFC 5737 TEST-NET-2", "host 198.51.100.7", "none"),
            c("RFC 5737 TEST-NET-3", "host 203.0.113.9", "none"),
            c("バージョン文字列", "installed v1.2.3.4 today", "none"),
            c("5 つ組 (8.8.8.8.1) は撃たない", "ver 8.8.8.8.1", "none"),
            c("CIDR 表記 (/24) は範囲の記述で撃たない", "route 8.8.8.0/24 via gw", "none"),
        ]
    }
    stop_pass!();
}

mod net_ipv6 {
    use super::*;

    fn cases() -> Vec<Case> {
        vec![
            c("global unicast IPv6", "aaaa 2606:4700:4700::1111 bbbb", "ipv6"),
            c("Google DNS IPv6", "nameserver 2001:4860:4860::8888", "ipv6"),
            c("単一ホストの /128 は撃つ", "host 2001:4860:4860::8888/128", "ipv6"),
            c("RFC 3849 文書用", "doc addr 2001:db8::1", "none"),
            c("link-local", "iface fe80::1", "none"),
            c("ULA", "ula fd00::1", "none"),
            c("loopback IPv6", "bind ::1", "none"),
            c("multicast IPv6", "group ff02::1", "none"),
            c("C++/Rust 名前空間 abc::def", "use abc::def;", "none"),
            c("dead::beef", "marker dead::beef here", "none"),
            c("ISO 8601 タイムスタンプ", r#""installedAt": "2026-07-13T14:27:17.429Z""#, "none"),
            c("bash 部分文字列展開", r#"short_uuid="${target_uuid:0:8}""#, "none"),
            c("CIDR 表記 (2000::/3) は範囲の記述で撃たない", "global unicast is 2000::/3", "none"),
            c("CIDR 表記 (/32) は範囲の記述で撃たない", "prefix 2001:4860::/32", "none"),
        ]
    }
    stop_pass!();
}

mod email {
    use super::*;

    fn cases() -> Vec<Case> {
        vec![
            c("実メールアドレス", format!("contact {} for detail", mail()), "email"),
            c("mailto: 内のメールアドレス", format!("mailto:{}", mail()), "email"),
            c("SSH URL の git@github.com", "git clone git@github.com:example/repo.git ./repo", "none"),
            c("SSH URL の case パターン", "  git@github.com:*)", "none"),
            c("SSH URL の変数展開", r#"  repo="${url#git@github.com:}""#, "none"),
            c("noreply 系", "Co-Authored-By: Claude <noreply@anthropic.com>", "none"),
            c("noreply 系 (ハイフン付き、大文字小文字)", format!("From: Do-Not-Reply@{}{}", "fixture-corp", ".io"), "none"),
            c("RFC 2606 予約ドメイン", r#"git config user.email "test@example.com""#, "none"),
            c("RFC 2606 予約ドメイン (example.org)", "mail alice@example.org", "none"),
            c("極端に短い local + ラベル", r#"# credentials is "service_account demo-svc x@y.com","#, "none"),
        ]
    }
    stop_pass!();
}

mod cloud_credential {
    use super::*;

    fn cases() -> Vec<Case> {
        let sa_type = cat(&[r#""type""#, ": ", r#""service_account""#]);
        let sig = format!("{}%3D", gen(ALNUM, 44, 71));
        vec![
            c("実 AWS アカウント ID の ARN", format!("arn:aws:sts::{}:assumed-role/x", account()), "aws-arn-acct"),
            c("実 AWS アカウント ID の ARN (GovCloud)", format!("arn:aws-us-gov:iam::{}:user/x", account()), "aws-arn-acct"),
            c("AWS access key id", format!("aws_access_key_id = {}", akid("AKIA", 11)), "aws-akid"),
            c("GCP OAuth client secret", format!("client_secret: {}{}", cat(&["GOC", "SPX-"]), gen(URLSAFE, 28, 12)), "gcp-oauth-sec"),
            c("GCP SA JSON マーカー", format!("  {sa_type},"), "gcp-sa-json"),
            c("Azure SAS (sv= が sig= より前)", format!("https://x.blob.core.windows.net/c/b?sv=2021-06-08&se=2026-01-01&sig={sig}"), "azure-sas"),
            c("文書用アカウント ID (昇順連番)", "arn:aws:eks:us-east-1:123456789012:cluster/x", "none"),
            c("文書用アカウント ID (降順連番)", "arn:aws:iam::987654321098:role/x", "none"),
            c("文書用アカウント ID (低多様性)", "arn:aws:iam::111122223333:role/x", "none"),
            c("アカウント ID の無い ARN", "arn:aws:iam::aws:policy/ReadOnlyAccess", "none"),
            c("GUID (tenant/client id)", format!(r#"tenant_id = "{}""#, guid(81)), "none"),
            c("Azure SAS: sv= も core.windows.net も無い sig= は撃たない", format!("https://api.fixture-corp.io/x?sig={sig}"), "none"),
            c("GCP SA JSON: type が service_account 以外は撃たない", r#"  "type": "authorized_user","#, "none"),
        ]
    }
    stop_pass!();
}

mod db_credential {
    use super::*;

    fn cases() -> Vec<Case> {
        let pw = gen(ALNUM, 12, 91);
        vec![
            c("password 付き接続 URL", format!("DSN=postgres://appuser:{pw}@db.example.test:5432/app"), "url"),
            c("password 付き接続 URL (mysql)", format!("DSN=mysql://appuser:{pw}@db.example.test:3306/app"), "url"),
            c("password 付き接続 URL (redis、user は default)", format!("REDIS=redis://default:{pw}@cache.example.test:6379/0"), "url"),
            c("password 付き接続 URL (mongodb+srv)", format!("URI=mongodb+srv://appuser:{pw}@c.example.test/app"), "url"),
            c("JDBC の password= パラメータ", format!("jdbc:postgresql://db.example.test:5432/app?user=app&password={pw}"), "jdbc"),
            c("DB の password 環境変数", format!("export PGPASSWORD={pw}"), "env"),
            c("DB の password 環境変数 (クォート付き)", format!(r#"POSTGRES_PASSWORD="{pw}""#), "env"),
            c("password 無しの接続 URL", "DSN=postgres://readonly@replica.example.test:5432/analytics", "none"),
            c("プレースホルダ password", "DSN=mysql://user:password@localhost:3306/dbname", "none"),
            c("変数展開の password", "DSN=postgres://user:${DB_PASSWORD}@host/db", "none"),
            c("検出器のパターンソース行", r#"    (r'(?i)(mysql://)\\S+:\\S+@', "MySQL Connection String"),"#, "none"),
            c("5 文字以下の password は撃たない", "DSN=postgres://appuser:Ab3dE@db.example.test/app", "none"),
            c("低エントロピーの password", "DSN=postgres://appuser:aaaaaaaaaa@db.example.test/app", "none"),
            c("JDBC のプレースホルダ password", "jdbc:postgresql://db.example.test/app?password=changeme", "none"),
            c("環境変数の変数展開", "DB_PASSWORD=${DB_PASSWORD}", "none"),
            c("環境変数のプレースホルダ", "POSTGRES_PASSWORD=password", "none"),
        ]
    }
    stop_pass!();
}

mod credential_generic {
    use super::*;

    fn cases() -> Vec<Case> {
        let v = gen(ALNUM, 24, 101);
        vec![
            // (?i) のスコープ事故で検出が silent に死ぬのを検知する回帰。必ず陽性ケースを置く
            c("汎用代入層の陽性", format!(r#"password: "{v}""#), "generic"),
            c("汎用代入層の陽性 (シングルクォート、api_key)", format!("api_key = '{v}'"), "generic"),
            c("汎用代入層の陽性 (大文字キー名)", format!(r#"CLIENT_SECRET="{v}""#), "generic"),
            c("プレースホルダ値", r#"api_key = "YOUR_API_KEY_HERE""#, "none"),
            c("変数展開値", r#"password = "${DB_PASSWORD}""#, "none"),
            c("低エントロピー値", r#"token = "aaaaaaaaaaaaaaaaaaaa""#, "none"),
            c("環境変数名そのもの", r#"secret = "SOME_ENV_VAR_NAME""#, "none"),
            c("15 文字以下の値は撃たない", format!(r#"secret = "{}""#, gen(ALNUM, 15, 102)), "none"),
            c("クォート無しの代入は撃たない", format!("token = {v}"), "none"),
            c("my_ で始まる値はプレースホルダ", format!(r#"password: "my_{}""#, gen(ALNUM, 20, 103)), "none"),
            c("example を含む値はプレースホルダ", format!(r#"api_key = "example{}""#, gen(ALNUM, 20, 104)), "none"),
        ]
    }
    stop_pass!();
}

// ---------------------------------------------------------------------------
// 検出ルール単体 (rule id まで見る)
// ---------------------------------------------------------------------------

mod detection_rules {
    use super::*;

    fn cases() -> Vec<Case> {
        let ya = cat(&["ya", "29."]);
        let ya_val = gen(URLSAFE, 28, 201);
        let (dtc, dts) = (cat(&["dt0", "c01"]), cat(&["dt0", "s01"]));
        let (dt_pub, dt_sec) = (gen(UPNUM, 24, 202), gen(UPNUM, 64, 203));
        let (h32, h40) = (gen(HEX, 32, 204), gen(HEX, 40, 205));
        let g = guid(206);
        let xs = gen(B64, 40, 207);
        let st = gen(B64, 120, 208);
        let stripe = gen(ALNUM, 24, 209);
        let sk = cat(&["sk_", "test_"]);
        let rk = cat(&["rk_", "test_"]);
        let (doc_sec, doc_st) = (aws_doc_secret(), aws_doc_session());
        vec![
            // 検知漏れだったルール (止める側)
            c("ya29 の OAuth access token", format!("Authorization: Bearer {ya}{ya_val}"), "gcp-oauth-token"),
            c("ya29 の内部に . を持つ形 (ya29.c.)", format!("token {ya}c.{ya_val}"), "gcp-oauth-token"),
            c("Dynatrace dt0c01 token (24 + 64)", format!("Authorization: Api-Token {dtc}.{dt_pub}.{dt_sec}"), "dynatrace"),
            c("Dynatrace dt0s01 token (24 + 64)", format!("TOKEN={dts}.{dt_pub}.{dt_sec}"), "dynatrace"),
            c("DD_API_KEY (環境変数の形、16 進 32 桁)", format!("DD_API_KEY={h32}"), "datadog-api-key"),
            c("DD_APP_KEY (環境変数の形、16 進 40 桁)", format!("DD_APP_KEY={h40}"), "datadog-app-key"),
            c("application_key (クォート付きの代入、40 桁)", format!(r#"application_key: "{h40}""#), "datadog-app-key"),
            c("Splunk HEC の Authorization header", format!("Authorization: Splunk {g}"), "splunk-hec"),
            c("AWS の XML <SecretAccessKey> (40 桁)", format!("<SecretAccessKey>{xs}</SecretAccessKey>"), "aws-secret-xml"),
            c("Stripe sk_test_", format!("STRIPE_KEY={sk}{stripe}"), "stripe"),
            c("Stripe rk_test_", format!("STRIPE_KEY={rk}{stripe}"), "stripe"),
            c("MAC のハイフン区切り (実 MAC)", "gw A4-83-E7-11-22-33 up", "mac"),
            c("対照: EXAMPLE を抜いた session token は止める", format!("aws_session_token={st}"), "aws-session-token"),
            c("対照: EXAMPLE を抜いた secret は止める", format!("aws_secret_access_key={xs}"), "aws-secret-key"),
            // 誤検知を避ける境界 (止めない側)
            c("AWS: EXAMPLE を含む secret はキー名付きでも通す", format!("aws_secret_access_key={}", replace_range(&xs, 20, 27, "EXAMPLE")), "none"),
            c("AWS: EXAMPLE を含む XML <SecretAccessKey> は通す", format!("<SecretAccessKey>{}</SecretAccessKey>", replace_range(&xs, 20, 27, "EXAMPLE")), "none"),
            c("AWS: EXAMPLE を含む session token は通す", format!("aws_session_token={}", replace_range(&st, 60, 67, "EXAMPLE")), "none"),
            c("AWS 公式 docs の secret access key (キー名付き)", format!("aws_secret_access_key={doc_sec}"), "none"),
            c("AWS 公式 docs の secret access key (XML)", format!("<SecretAccessKey>{doc_sec}</SecretAccessKey>"), "none"),
            c("AWS 公式 docs の session token", format!("aws_session_token={doc_st}"), "none"),
            c("Dynatrace: 桁数違いは撃たない", format!("Api-Token {dtc}.abc123.abcdefjhij1234567890"), "none"),
            c("Datadog: 31 桁の 16 進は撃たない", format!("DD_API_KEY={}", &h32[..31]), "none"),
            c("Datadog: 33 桁の 16 進は撃たない", format!("DD_API_KEY={h32}a"), "none"),
            c("Datadog: 41 桁の 16 進は撃たない (桁数の境界)", format!("DD_APP_KEY={h40}a"), "none"),
            c("Datadog: 低エントロピーの 16 進 (全部 0)", "DD_API_KEY=00000000000000000000000000000000", "none"),
            c("Splunk: header 形でない GUID は撃たない", format!("tenant {g}"), "none"),
            c("ya29 を含む短い語は撃たない", "build ya29.ok", "none"),
            c("Splunk: 低エントロピーの GUID (全部 0)", "Authorization: Splunk 00000000-0000-0000-0000-000000000000", "none"),
            c("Datadog app key: 低エントロピーの 16 進 (全部 0)", "DD_APP_KEY=0000000000000000000000000000000000000000", "none"),
        ]
    }
    stop_pass!();
}

mod kept_exclusions {
    use super::*;

    // 既存の除外が新ルール追加後も残っていること (RFC 2606 / 5737 / 3849 / 7042)
    fn cases() -> Vec<Case> {
        vec![
            c("RFC 2606 の予約 TLD (.test)", "mail user@foo.test", "none"),
            c("RFC 2606 の予約 TLD (.invalid)", "mail user@host.invalid", "none"),
            c("RFC 2606 の予約 TLD (.localhost)", "mail user@app.localhost", "none"),
            c("RFC 5737 TEST-NET-2", "host 198.51.100.7", "none"),
            c("RFC 5737 TEST-NET-3", "host 203.0.113.9", "none"),
            c("RFC 3849 文書用 IPv6", "host 2001:db8::1", "none"),
            c("RFC 7042 文書用 MAC (コロン、末尾 ff)", "mac 00:00:5e:00:53:ff", "none"),
        ]
    }
    only_pass!();
}

// 元の test に無かったルール (gh / slack / llm / pem / jwt / npm / gcp / azure の各ルール)
mod prefix_rules {
    use super::*;

    fn cases() -> Vec<Case> {
        let gh = cat(&["gh", "p_"]);
        let gho = cat(&["gh", "o_"]);
        let pat = cat(&["github", "_pat_"]);
        let slack = cat(&["xo", "xb-"]);
        let sk = cat(&["sk", "-"]);
        let ant = cat(&["sk", "-ant-"]);
        let pem = |kind: &str| cat(&["-----BEGIN", kind, " PRIVATE", " KEY-----"]);
        let jwt = |a: u64| format!("{}{}.{}{}.{}", cat(&["ey", "J"]), gen(URLSAFE, 20, a), cat(&["ey", "J"]), gen(URLSAFE, 20, a + 1), gen(URLSAFE, 24, a + 2));
        let npm = cat(&["npm", "_"]);
        let aiza = cat(&["AI", "za"]);
        let refresh = cat(&["1/", "/0"]);
        let acct_key = cat(&["Account", "Key="]);
        let st = gen(B64, 120, 313);
        let pkid = cat(&[r#""private"#, r#"_key_id""#, ": ", "\""]);
        vec![
            c("aws-akid: AKIA", format!("key {}", akid("AKIA", 301)), "aws-akid"),
            c("aws-akid: ASIA (STS の一時 key)", format!("key {}", akid("ASIA", 302)), "aws-akid"),
            c("aws-akid: ABIA", format!("key {}", akid("ABIA", 303)), "aws-akid"),
            c("aws-akid: ACCA", format!("key {}", akid("ACCA", 304)), "aws-akid"),
            c("gh-token: ghp_ (36 桁の英数字)", format!("GH={gh}{}", gen(ALNUM, 36, 305)), "gh-token"),
            c("gh-token: gho_ (40 桁の英数字)", format!("GH={gho}{}", gen(ALNUM, 40, 306)), "gh-token"),
            c("gh-pat: github_pat_ (82 桁)", format!("GH={pat}{}", gen(ALNUM_US, 82, 307)), "gh-pat"),
            c("slack: xoxb-", format!("SLACK={slack}{}-{}", gen(DIGITS, 12, 308), gen(ALNUM, 24, 309)), "slack"),
            c("llm-key: sk-", format!("KEY={sk}{}", gen(ALNUM, 40, 310)), "llm-key"),
            c("llm-key: sk-ant-", format!("KEY={ant}{}", gen(URLSAFE, 40, 311)), "llm-key"),
            c("stripe: sk_live_", format!("KEY={}{}", cat(&["sk_", "live_"]), gen(ALNUM, 24, 312)), "stripe"),
            c("pem: RSA PRIVATE KEY の header", pem(" RSA"), "pem"),
            c("pem: 種別無しの PRIVATE KEY の header", pem(""), "pem"),
            c("jwt: 3 部構成", format!("Authorization: Bearer {}", jwt(320)), "jwt"),
            c("npm: npm_ (36 桁)", format!("NPM={npm}{}", gen(ALNUM, 36, 321)), "npm"),
            c("aws-session-token: キー名付き", format!("aws_session_token={st}"), "aws-session-token"),
            c("gcp-api-key: AIza (35 桁)", format!("key={aiza}{}", gen(URLSAFE, 35, 322)), "gcp-api-key"),
            c("gcp-refresh: 1//0 の refresh token", format!("refresh {refresh}{}", gen(URLSAFE, 40, 323)), "gcp-refresh"),
            c("gcp-sa-json: private_key_id (16 進 40 桁)", format!("  {pkid}{}\",", gen(HEX, 40, 324)), "gcp-sa-json"),
            c("azure-storage: AccountKey= (80 桁以上)", format!("Conn;{acct_key}{}", gen(B64, 88, 325)), "azure-storage"),
            c("azure-client-secret: AZURE_CLIENT_SECRET", format!("AZURE_CLIENT_SECRET={}", gen(AZ, 40, 326)), "azure-client-secret"),
            c("azure-adv2: 8Q~ を含む client secret", format!("secret {}8Q~{}", gen(ALNUM, 3, 327), gen(AZ, 34, 328)), "azure-adv2"),
            c("aws-akid: 15 桁は撃たない", format!("key {}", &akid("AKIA", 301)[..19]), "none"),
            c("aws-akid: 17 桁は撃たない (後ろの境界)", format!("key {}A", akid("AKIA", 301)), "none"),
            c("aws-akid: 小文字は撃たない", format!("key {}", akid("AKIA", 301).to_lowercase()), "none"),
            c("gh-token: 35 桁は撃たない", format!("GH={gh}{}", gen(ALNUM, 35, 305)), "none"),
            c("gh-pat: 81 桁は撃たない", format!("GH={pat}{}", gen(ALNUM_US, 81, 307)), "none"),
            c("slack: 10 桁未満は撃たない", format!("SLACK={slack}short"), "none"),
            c("slack: xoxz- は撃たない", format!("SLACK={}{}", cat(&["xo", "xz-"]), gen(ALNUM, 24, 309)), "none"),
            c("llm-key: 短い値は撃たない", format!("KEY={sk}short"), "none"),
            c("llm-key: 語の途中の sk- は撃たない", "risk-assessment-framework-document", "none"),
            c("pem: 公開鍵 / 証明書は撃たない", "-----BEGIN CERTIFICATE-----", "none"),
            c("jwt: 2 部構成は撃たない", format!("t {}{}.{}{}", cat(&["ey", "J"]), gen(URLSAFE, 20, 320), cat(&["ey", "J"]), gen(URLSAFE, 20, 321)), "none"),
            c("npm: 35 桁は撃たない", format!("NPM={npm}{}", gen(ALNUM, 35, 321)), "none"),
            c("npm: 37 桁は撃たない (後ろの境界)", format!("NPM={npm}{}", gen(ALNUM, 37, 321)), "none"),
            c("gcp-api-key: 34 桁は撃たない", format!("key={aiza}{}", gen(URLSAFE, 34, 322)), "none"),
            c("gcp-oauth-sec: 27 桁は撃たない", format!("client_secret: {}{}", cat(&["GOC", "SPX-"]), gen(URLSAFE, 27, 12)), "none"),
            c("azure-storage: 79 桁は撃たない", format!("Conn;{acct_key}{}", gen(B64, 79, 325)), "none"),
            c("azure-client-secret: プレースホルダは撃たない", "AZURE_CLIENT_SECRET=your-client-secret-goes-here-0000", "none"),
            c("azure-adv2: 33 桁は撃たない", format!("secret {}8Q~{}", gen(ALNUM, 3, 327), gen(AZ, 33, 328)), "none"),
        ]
    }
    stop_pass!();
}

// ---------------------------------------------------------------------------
// 行内 pragma / policy の exempt / policy 未登録の repo
// ---------------------------------------------------------------------------

mod pragma {
    use super::*;

    fn cases() -> Vec<Case> {
        vec![
            c("行内 pragma", "gw=a4:83:e7:11:22:33  # secret-scan-gate:allow", "none"),
            c("行内 pragma (credential でも飛ばす)", format!("key = {} secret-scan-gate:allow", akid("AKIA", 11)), "none"),
            c("対照: pragma 無しの同じ行は止める", "gw=a4:83:e7:11:22:33", "mac"),
        ]
    }
    stop_pass!();
}

mod policy_exempt {
    use super::*;
    const D: &str = "example/docs";

    fn cases() -> Vec<Case> {
        let akid = akid("AKIA", 11);
        let v = gen(ALNUM, 24, 101);
        vec![
            // paths x classes (class のワイルドカード)
            c("docs/** の net-* 免除: MAC", "gateway=a4:83:e7:11:22:33", "none").at(D, "docs/guide.md"),
            c("docs/** の net-* 免除: IPv4", "resolver=8.8.8.8", "none").at(D, "docs/guide.md"),
            c("docs/** の net-* 免除: IPv6", "nameserver 2001:4860:4860::8888", "none").at(D, "docs/guide.md"),
            c("fnmatch の * は / を越える (docs/a/b/c.md)", "gateway=a4:83:e7:11:22:33", "none").at(D, "docs/a/b/c.md"),
            c("免除 class に含まれない email は止める", format!("contact {}", mail()), "email").at(D, "docs/guide.md"),
            c("免除 path でも credential は止める", format!("aws_access_key_id = {akid}"), "aws-akid").at(D, "docs/guide.md"),
            c("免除 path の外は止める", "gateway=a4:83:e7:11:22:33", "mac").at(D, "src/guide.md"),
            c("path の接頭辞だけ似ていても免除しない (docs-old/)", "gateway=a4:83:e7:11:22:33", "mac").at(D, "docs-old/guide.md"),
            c("免除は repo ごと (exempt 無しの repo では止める)", "gateway=a4:83:e7:11:22:33", "mac").at("example/repo", "docs/guide.md"),
            // rule 単位 (`class/rule`)
            c("rule 単位の免除: net-mac/mac は MAC を通す", "gateway=a4:83:e7:11:22:33", "none").at(D, "notes/a.md"),
            c("rule 単位の免除: 深い path でも通す", "gateway=a4:83:e7:11:22:33", "none").at(D, "notes/x/y/a.md"),
            c("rule 単位の免除: 同じ path の IPv4 は止める", "resolver=8.8.8.8", "ipv4").at(D, "notes/a.md"),
            c("rule 単位の免除: credential/generic は generic を通す", format!(r#"password: "{v}""#), "none").at(D, "extra/a.md"),
            c("rule 単位の免除: 同じ class の別 rule は止める", format!("aws_access_key_id = {akid}"), "aws-akid").at(D, "extra/a.md"),
            c("rule 単位の指定はワイルドカードを解さない", "gateway=a4:83:e7:11:22:33", "mac").at(D, "wild/a.md"),
        ]
    }
    stop_pass!();
}

// ---------------------------------------------------------------------------
// 差分の読み取り
// ---------------------------------------------------------------------------

// (説明, repo, diff, 期待する (file, line) の列)
type DiffCase = (&'static str, &'static str, &'static str, Vec<(&'static str, u64)>);

fn diff_ng(cases: &[DiffCase], want_found: bool) {
    let t = Tmp::new();
    let policy = t.file("policy.json", &policy_json().to_string());
    let (mut n, mut ng) = (0, Vec::new());
    for (desc, repo, diff, want) in cases.iter().filter(|k| !k.3.is_empty() == want_found) {
        n += 1;
        let out = scan(&policy, repo, diff);
        let got: Option<Vec<(String, u64)>> = findings(&out).map(|f| {
            f.iter().map(|x| (x["file"].as_str().unwrap_or("?").to_string(), x["line"].as_u64().unwrap_or(u64::MAX))).collect()
        });
        let want: Vec<(String, u64)> = want.iter().map(|(f, l)| (f.to_string(), *l)).collect();
        if got.as_ref() != Some(&want) {
            ng.push(format!("  NG {desc}: 期待 {want:?} / 実際 {got:?} stderr={}", out.stderr.trim()));
        }
    }
    assert!(n > 0, "走らせるケースが 0 件");
    assert!(ng.is_empty(), "{} 件中 {} 件失敗\n{}", n, ng.len(), ng.join("\n"));
}

// 8.8.8.8 / 8.8.4.4 (global IPv4) を検出の目印にして、(file, line) だけを見る
fn diff_cases() -> Vec<DiffCase> {
    let r = "example/repo";
    vec![
        ("削除のみの差分 (+++ /dev/null)", r, "--- a/f.md\n+++ /dev/null\n@@ -1 +0,0 @@\n-8.8.8.8\n", vec![]),
        ("+++ /dev/null の後の + 行は追跡しない", r, "+++ /dev/null\n@@ -0,0 +1 @@\n+8.8.8.8\n", vec![]),
        ("削除行 (-) は見ない", r, "+++ b/f.md\n@@ -1 +0,0 @@\n-8.8.8.8\n", vec![]),
        ("文脈行 (空白始まり) は見ない", r, "+++ b/f.md\n@@ -1 +1 @@\n 8.8.8.8\n", vec![]),
        ("+++ ヘッダの無い追加行は見ない", r, "+8.8.8.8\n", vec![]),
        ("空の差分", r, "", vec![]),
        ("追加行 1 件", r, "+++ b/f.md\n@@ -0,0 +1 @@\n+8.8.8.8\n", vec![("f.md", 1)]),
        ("a/ 接頭辞も剥がす", r, "+++ a/f.md\n@@ -0,0 +1 @@\n+8.8.8.8\n", vec![("f.md", 1)]),
        ("CRLF の差分", r, "+++ b/f.md\r\n@@ -0,0 +1 @@\r\n+8.8.8.8\r\n", vec![("f.md", 1)]),
        (
            "複数ファイル",
            r,
            "diff --git a/a.md b/a.md\n+++ b/a.md\n@@ -0,0 +1 @@\n+8.8.8.8\ndiff --git a/d/b.md b/d/b.md\n+++ b/d/b.md\n@@ -0,0 +3 @@\n+x\n+y\n+8.8.4.4\n",
            vec![("a.md", 1), ("d/b.md", 5)],
        ),
        ("hunk ごとに行番号を取り直す", r, "+++ b/f.md\n@@ -1,0 +5,2 @@\n+x\n+8.8.8.8\n@@ -9 +20 @@\n+8.8.4.4\n", vec![("f.md", 6), ("f.md", 20)]),
        ("hunk が無ければ行番号は 0 から数える", r, "+++ b/f.md\n+x\n+8.8.8.8\n", vec![("f.md", 1)]),
        // Python 実装の癖 (内容が `++ ` で始まる追加行は +++ ヘッダと誤認して path が切り替わる) はそのまま再現が仕様
        (
            "内容が ++ で始まる追加行を +++ ヘッダと誤認する癖",
            r,
            "+++ b/f.md\n@@ -0,0 +1,2 @@\n+8.8.8.8\n+++ b/other.md\n+8.8.4.4\n",
            vec![("f.md", 1), ("other.md", 2)],
        ),
        ("内容が ++ だけの追加行は無視され行番号も進まない", r, "+++ b/f.md\n@@ -0,0 +1 @@\n+++\n+8.8.8.8\n", vec![("f.md", 1)]),
        ("pragma の行だけ飛ばし、次の行は見る", r, "+++ b/f.md\n@@ -0,0 +1,2 @@\n+8.8.8.8 # secret-scan-gate:allow\n+8.8.4.4\n", vec![("f.md", 2)]),
        (
            "exempt は差分内のファイルごとに効く",
            "example/docs",
            "+++ b/docs/a.md\n@@ -0,0 +1 @@\n+8.8.8.8\n+++ b/src/a.md\n@@ -0,0 +1 @@\n+8.8.8.8\n",
            vec![("src/a.md", 1)],
        ),
    ]
}

#[test]
fn diff_reading_nothing_to_report() {
    diff_ng(&diff_cases(), false);
}

#[test]
fn diff_reading_reports_file_and_line() {
    diff_ng(&diff_cases(), true);
}

// ---------------------------------------------------------------------------
// 出力の形とマスク
// ---------------------------------------------------------------------------

#[test]
fn finding_has_file_line_class_rule_value() {
    let t = Tmp::new();
    let p = t.file("policy.json", &policy_json().to_string());
    let out = scan(&p, "example/repo", "+++ b/d/f.md\n@@ -0,0 +7 @@\n+resolver=8.8.8.8\n");
    let want = json!({"findings": [{"file": "d/f.md", "line": 7, "class": "net-ipv4", "rule": "ipv4", "value": "8.8.8.8"}], "scanned": true});
    let got: Value = serde_json::from_str(&out.stdout).unwrap_or(Value::Null);
    assert_eq!((out.code, got), (Some(0), want), "stderr={}", out.stderr);
}

/// (説明, 追加行, class, rule, scanner が報告する値)。値は credential 系なら先頭 6 文字 + … になる
fn mask_cases() -> Vec<(&'static str, String, &'static str, &'static str, String)> {
    let key = akid("AKIA", 401);
    let ya = format!("{}{}", cat(&["ya", "29."]), gen(URLSAFE, 28, 402));
    let pw = gen(ALNUM, 12, 403);
    let short_pw = gen(ALNUM, 6, 404);
    let wide_pw = "あいうえおかきくけこ".to_string();
    // 以下の URL は password を `{}` で差す。`{short_pw}` のような名前付きの形だと、このファイル自身を
    // 追加行として検査する側が「6 文字以上の password」と読んで止めてしまう
    let g = gen(ALNUM, 24, 405);
    vec![
        ("aws-akid は先頭 6 文字 + …", format!("key {key}"), "credential", "aws-akid", key),
        ("gcp-oauth-token (cloud-credential) も先頭 6 文字 + …", format!("Bearer {ya}"), "cloud-credential", "gcp-oauth-token", ya),
        ("db-credential は password 部分を先頭 6 文字 + …", format!("DSN=postgres://appuser:{pw}@db.example.test/app"), "db-credential", "url", pw),
        ("6 文字ちょうどの値は … だけ", format!("DSN=postgres://appuser:{}@db.example.test/app", short_pw), "db-credential", "url", short_pw),
        ("多バイト文字は文字単位で 6 文字", format!("DSN=postgres://appuser:{}@db.example.test/app", wide_pw), "db-credential", "url", wide_pw),
        ("generic も値の先頭 6 文字 + …", format!(r#"password: "{g}""#), "credential", "generic", g),
    ]
}

#[test]
fn credential_values_are_masked() {
    let t = Tmp::new();
    let p = t.file("policy.json", &policy_json().to_string());
    let mut ng = Vec::new();
    for (desc, line, class, rule, full) in mask_cases() {
        let head: String = full.chars().take(6).collect();
        let want = if full.chars().count() > 6 { format!("{head}…") } else { "…".to_string() };
        let out = scan(&p, "example/repo", &format!("+++ b/f.md\n@@ -0,0 +1 @@\n+{line}\n"));
        let got = findings(&out).and_then(|f| f.first().cloned());
        let ok = got.as_ref().is_some_and(|f| f["class"] == class && f["rule"] == rule && f["value"] == want);
        if !ok {
            ng.push(format!("  NG {desc}: 期待 {class}/{rule} 値 {want} / 実際 {}", got.map_or("検出なし".into(), |f| f.to_string())));
        }
    }
    assert!(ng.is_empty(), "{} 件失敗\n{}", ng.len(), ng.join("\n"));
}

#[test]
fn identifier_values_are_not_masked() {
    let t = Tmp::new();
    let p = t.file("policy.json", &policy_json().to_string());
    let m = mail();
    let cases = [
        ("net-mac", "mac", "a4:83:e7:11:22:33", "gw a4:83:e7:11:22:33 up".to_string()),
        ("net-mac", "mac", "A4-83-E7-11-22-33", "gw A4-83-E7-11-22-33 up".to_string()),
        ("net-ipv4", "ipv4", "8.8.8.8", "resolver=8.8.8.8".to_string()),
        ("net-ipv6", "ipv6", "2001:4860:4860::8888", "nameserver 2001:4860:4860::8888".to_string()),
        ("email", "email", m.as_str(), format!("contact {m} now")),
    ];
    let mut ng = Vec::new();
    for (class, rule, value, line) in cases {
        let out = scan(&p, "example/repo", &format!("+++ b/f.md\n@@ -0,0 +1 @@\n+{line}\n"));
        let got = findings(&out).and_then(|f| f.first().cloned());
        let ok = got.as_ref().is_some_and(|f| f["class"] == class && f["rule"] == rule && f["value"] == value);
        if !ok {
            ng.push(format!("  NG {class}/{rule}: 期待 値 {value} そのまま / 実際 {}", got.map_or("検出なし".into(), |f| f.to_string())));
        }
    }
    assert!(ng.is_empty(), "{} 件失敗\n{}", ng.len(), ng.join("\n"));
}

// ---------------------------------------------------------------------------
// CLI の契約 (scanned=false / 引数不正 / policy 不正)
// ---------------------------------------------------------------------------

const TRIGGER_DIFF: &str = "+++ b/f.md\n@@ -0,0 +1 @@\n+resolver=8.8.8.8\n";

fn assert_json(out: &Out, want: Value) {
    let got: Value = serde_json::from_str(&out.stdout).unwrap_or(Value::Null);
    assert_eq!((out.code, got), (Some(0), want), "stderr={}", out.stderr);
}

#[test]
fn unlisted_repo_is_not_scanned() {
    let t = Tmp::new();
    let p = t.file("policy.json", &policy_json().to_string());
    assert_json(&scan(&p, "example/unlisted", TRIGGER_DIFF), json!({"findings": [], "scanned": false}));
}

#[test]
fn policy_without_repos_scans_nothing() {
    let t = Tmp::new();
    let p = t.file("policy.json", "{}");
    assert_json(&scan(&p, "example/repo", TRIGGER_DIFF), json!({"findings": [], "scanned": false}));
}

#[test]
fn null_repo_entry_is_not_scanned() {
    let t = Tmp::new();
    let p = t.file("policy.json", r#"{"repos": {"example/repo": null}}"#);
    assert_json(&scan(&p, "example/repo", TRIGGER_DIFF), json!({"findings": [], "scanned": false}));
}

#[test]
fn empty_diff_is_scanned_without_findings() {
    let t = Tmp::new();
    let p = t.file("policy.json", &policy_json().to_string());
    assert_json(&scan(&p, "example/repo", ""), json!({"findings": [], "scanned": true}));
}

#[test]
fn equals_form_arguments_are_accepted() {
    let t = Tmp::new();
    let p = t.file("policy.json", &policy_json().to_string());
    let out = run_cli(&[&format!("--policy={}", p.display()), "--repo=example/repo"], "");
    assert_json(&out, json!({"findings": [], "scanned": true}));
}

#[test]
fn bad_arguments_exit_2() {
    let t = Tmp::new();
    let p = t.file("policy.json", &policy_json().to_string());
    let p = p.to_str().unwrap();
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("引数無し", vec![]),
        ("--repo が無い", vec!["--policy", p]),
        ("--policy が無い", vec!["--repo", "example/repo"]),
        ("--repo の値が無い", vec!["--policy", p, "--repo"]),
        ("未知の引数", vec!["--policy", p, "--repo", "example/repo", "--unknown", "1"]),
    ];
    let ng: Vec<String> = cases
        .iter()
        .filter_map(|(desc, args)| {
            let out = run_cli(args, TRIGGER_DIFF);
            // 偽の「検出なし」を stdout に出さないこと (呼び出し側は exit 2 を fail-close で扱う)
            (out.code != Some(2) || !out.stdout.is_empty()).then(|| format!("  NG {desc}: exit {:?} stdout={:?}", out.code, out.stdout))
        })
        .collect();
    assert!(ng.is_empty(), "{} 件失敗\n{}", ng.len(), ng.join("\n"));
}

#[test]
fn unreadable_policy_exits_2() {
    let t = Tmp::new();
    let missing = t.0.join("none.json");
    let cases = [
        ("policy が存在しない", missing),
        ("policy が空", t.file("empty.json", "")),
        ("policy が不正な JSON", t.file("broken.json", "{\"repos\": ")),
    ];
    let ng: Vec<String> = cases
        .iter()
        .filter_map(|(desc, p)| {
            let out = scan(p, "example/repo", TRIGGER_DIFF);
            (out.code != Some(2) || !out.stdout.is_empty()).then(|| format!("  NG {desc}: exit {:?} stdout={:?}", out.code, out.stdout))
        })
        .collect();
    assert!(ng.is_empty(), "{} 件失敗\n{}", ng.len(), ng.join("\n"));
}

// policy の形が想定と違うときは黙って通さず exit 2 (fail-close)。Python 実装はここで例外終了 (exit 1) するので、
// 契約として決めた Rust 側だけを見る
#[test]
fn malformed_policy_shape_exits_2() {
    if reference_mode() {
        return;
    }
    let t = Tmp::new();
    let cases = [
        ("policy が object でない", "[]".to_string()),
        ("repos が object でない", r#"{"repos": []}"#.to_string()),
        ("repo の設定が object でない", r#"{"repos": {"example/repo": []}}"#.to_string()),
        ("exempt が配列でない", r#"{"repos": {"example/repo": {"exempt": {}}}}"#.to_string()),
        ("exempt の要素が object でない", r#"{"repos": {"example/repo": {"exempt": ["x"]}}}"#.to_string()),
        ("exempt の paths が配列でない", r#"{"repos": {"example/repo": {"exempt": [{"paths": "docs/**"}]}}}"#.to_string()),
        ("entropy が object でない", r#"{"repos": {"example/repo": {}}, "entropy": 3}"#.to_string()),
        ("entropy の値が数値でない", r#"{"repos": {"example/repo": {}}, "entropy": {"generic": "3"}}"#.to_string()),
    ];
    let ng: Vec<String> = cases
        .iter()
        .enumerate()
        .filter_map(|(i, (desc, body))| {
            let p = t.file(&format!("p{i}.json"), body);
            let out = scan(&p, "example/repo", TRIGGER_DIFF);
            (out.code != Some(2) || !out.stdout.is_empty()).then(|| format!("  NG {desc}: exit {:?} stdout={:?}", out.code, out.stdout))
        })
        .collect();
    assert!(ng.is_empty(), "{} 件失敗\n{}", ng.len(), ng.join("\n"));
}
