// secret-scan のルール表と判定関数。Python 実装 (build_rules とその上のヘルパー) の移植。
//
// ルール表は Python 実装と同じ順序で並べる (finditer の報告順が findings の順序になる)。
//
// 正規表現を移すときの注意 (fancy-regex は Python の re と Unicode の扱いが違う。全部差分検証で実測した):
//   - `\s`: Python は str.isspace() で `\x1c`-`\x1f` を含むが、Rust の `\s` は含まない。
//     パターン中の `<S>` が `\s\x1c-\x1f` に置換されるので、文字クラスの中 (`[<S>]` / `[^<S>...]`) に書く
//   - `\d`: どちらも既定で Unicode の Nd 全般に当たるが、`(?-u)` や `(?a)` を付けると ASCII に狭まる。
//     付けずにそのまま移す
//   - `(?i)`: ASCII 語の `i` と文字クラスの英字は、Python だけ U+0130 (İ) と U+0131 (ı) にも当たる
//     (Rust の単純大文字小文字畳み込みには含まれない)。U+017F (ſ) と U+212A (K) は両者とも同じ。
//     パターン中の `<X>` が `\x{130}\x{131}` に置換されるので、(?i) 下の `i` は `[i<X>]`、
//     英字を含む文字クラスは末尾に `<X>` を足す
//   - Python の `(?i:...)` スコープ付きフラグは fancy-regex でも書ける。大文字固定の正規表現に
//     `(?i)` を掛けてはいけない (ENV_NAME_RE)
//   - Python の `$` は末尾の改行にも当たるが、走査する行は改行を含まないので差は出ない

use crate::secret_scan::{shannon_entropy, Entropy};
use fancy_regex::{Captures, Regex};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::rc::Rc;
use std::sync::OnceLock;

pub struct Rule {
    pub class: &'static str,
    pub id: &'static str,
    pub re: Regex,
    /// 検出とみなすなら報告値を返す。見送るなら None。第 2 引数は行全体 (Python の `m.string`)。
    pub check: Box<dyn Fn(&Captures<str>, &str) -> Option<String>>,
}

type Check = Box<dyn Fn(&Captures<str>, &str) -> Option<String>>;

fn re(pattern: &str) -> Regex {
    let p = pattern.replace("<S>", r"\s\x1c-\x1f").replace("<X>", r"\x{130}\x{131}");
    Regex::new(&p).unwrap_or_else(|e| panic!("invalid rule pattern {pattern}: {e}"))
}

// --- 定数 ---

// RFC 7042 §2.1.2 が文書用に予約する EUI-48 の unicast 範囲
const MAC_DOC_PREFIX: &str = "00:00:5e:00:53:";

// RFC 2606 が予約する TLD (§2) と SLD (§3)
const RESERVED_TLDS: [&str; 4] = ["test", "example", "invalid", "localhost"];
const RESERVED_DOMAINS: [&str; 3] = ["example.com", "example.net", "example.org"];

// SSH の login user として現れる `git@<host>` を除外するための既知ホスト
const VCS_SSH_HOSTS: [&str; 6] =
    ["github.com", "gitlab.com", "bitbucket.org", "ssh.dev.azure.com", "git.sr.ht", "codeberg.org"];

// 値が「秘密の名前」そのものの語なら、実値ではなくプレースホルダとみなす (エントロピーだけでは落ちない)
fn placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        re(concat!(
            r"(?i:^\$|^\{\{|^<|^x{4,}$|^\*{3,}$|your|my_|dummy|sample|example|changeme|",
            r"placeholder|redact|removed|h[i<X>]dden|^test$|fake|^\.\.\.|",
            r"^(?:pass(?:wd|word)?|pwd|secret|token|credent[i<X>]al|adm[i<X>]n|root|user(?:name)?)$)"
        ))
    })
}

// 値が環境変数名そのものの形。大文字固定なので大文字小文字を畳んではいけない
fn is_env_name(v: &str) -> bool {
    let mut cs = v.chars();
    cs.next().is_some_and(|c| c.is_ascii_uppercase()) && cs.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn is_weak_value(value: &str) -> bool {
    placeholder_re().is_match(value).unwrap_or(false) || is_env_name(value) || value.contains('\\')
}

// --- IP の is_global ---
//
// 基準は Python 3.14 の ipaddress。Rust の std の is_global は unstable で version も違うので、
// 予約レンジの表を自前で持つ。表は ipaddress の _private_networks / _private_networks_exceptions /
// _public_network をそのまま写した (3.9 では 192.0.0.0/29 だったり例外が無かったりして version で食い違う)

const V4_PRIVATE: [&str; 14] = [
    "0.0.0.0/8",
    "10.0.0.0/8",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.0.0.170/31",
    "192.0.2.0/24",
    "192.168.0.0/16",
    "198.18.0.0/15",
    "198.51.100.0/24",
    "203.0.113.0/24",
    "240.0.0.0/4",
    "255.255.255.255/32",
];
const V4_PRIVATE_EXCEPT: [&str; 2] = ["192.0.0.9/32", "192.0.0.10/32"];
// 共有アドレス空間。is_private は False だが is_global も False
const V4_SHARED: &str = "100.64.0.0/10";

const V6_PRIVATE: [&str; 11] = [
    "::1/128",
    "::/128",
    "::ffff:0:0/96",
    "64:ff9b:1::/48",
    "100::/64",
    "2001::/23",
    "2001:db8::/32",
    "2002::/16",
    "3fff::/20",
    "fc00::/7",
    "fe80::/10",
];
const V6_PRIVATE_EXCEPT: [&str; 6] =
    ["2001:1::1/128", "2001:1::2/128", "2001:3::/32", "2001:4:112::/48", "2001:20::/28", "2001:30::/28"];

fn in_net4(ip: u32, cidr: &str) -> bool {
    let (a, p) = cidr.split_once('/').expect("cidr");
    let (base, p) = (u32::from(a.parse::<Ipv4Addr>().expect("cidr")), p.parse::<u32>().expect("cidr"));
    ip >> (32 - p) == base >> (32 - p)
}

fn in_net6(ip: u128, cidr: &str) -> bool {
    let (a, p) = cidr.split_once('/').expect("cidr");
    let (base, p) = (u128::from(a.parse::<Ipv6Addr>().expect("cidr")), p.parse::<u32>().expect("cidr"));
    ip >> (128 - p) == base >> (128 - p)
}

fn is_global4(ip: u32) -> bool {
    let private = V4_PRIVATE.iter().any(|n| in_net4(ip, n)) && !V4_PRIVATE_EXCEPT.iter().any(|n| in_net4(ip, n));
    !in_net4(ip, V4_SHARED) && !private
}

fn is_global6(ip: u128) -> bool {
    // IPv4-mapped は下位 32 bit の IPv4 として判定する (ipaddress の ipv4_mapped)
    if ip >> 32 == 0xFFFF {
        return is_global4(ip as u32);
    }
    !(V6_PRIVATE.iter().any(|n| in_net6(ip, n)) && !V6_PRIVATE_EXCEPT.iter().any(|n| in_net6(ip, n)))
}

// --- 判定関数 ---

// arp(8) が先頭ゼロを落とす形を 2 桁ゼロ詰めへ、ハイフン区切りをコロン区切りへ正規化する
fn normalize_mac(raw: &str) -> String {
    raw.split([':', '-']).map(|p| format!("{p:0>2}").to_ascii_lowercase()).collect::<Vec<_>>().join(":")
}

fn check_mac(raw: &str) -> bool {
    let mac = normalize_mac(raw);
    let Some(first) = mac.split(':').next().and_then(|o| u8::from_str_radix(o, 16).ok()) else { return false };
    // 第 1 オクテットの LSB が 1 なら multicast/broadcast。実環境の識別子ではない
    if first & 1 == 1 || mac.starts_with(MAC_DOC_PREFIX) {
        return false;
    }
    mac != "00:00:00:00:00:00" && mac != "ff:ff:ff:ff:ff:ff"
}

fn check_ipv4(raw: &str) -> bool {
    raw.parse::<Ipv4Addr>().is_ok_and(|a| is_global4(a.into()))
}

fn check_ipv6(raw: &str) -> bool {
    let Ok(a) = raw.split('%').next().unwrap_or("").parse::<Ipv6Addr>() else { return false };
    let ip = u128::from(a);
    // IANA が global unicast に割り当てている唯一の範囲 (2000::/3, RFC 4291 §2.4)。
    // is_global 単独では abc::def や dead::beef、multicast、NAT64 を撃ってしまうため AND を取る
    is_global6(ip) && ip >> 125 == 1
}

fn check_aws_account(account: &str) -> bool {
    // 123456789012 のような昇順・降順連番の部分文字列は AWS 文書の慣用値
    if "01234567890123456789".contains(account) || "98765432109876543210".contains(account) {
        return false;
    }
    // 111122223333 のように相異なる数字が極端に少ないものも慣用値
    account.chars().collect::<std::collections::HashSet<_>>().len() > 4
}

fn check_email(local: &str, domain: &str, rest: &str) -> bool {
    let domain_lower = domain.to_lowercase();
    // SSH URL 形 (`git@github.com:owner/repo`)。直後がパス区切りの `:`
    let mut rc = rest.chars();
    if rc.next() == Some(':') && rc.next().is_some_and(|c| c.is_ascii_alphanumeric() || "._~*}-".contains(c)) {
        return false;
    }
    if local.to_lowercase() == "git" && VCS_SSH_HOSTS.contains(&domain_lower.as_str()) {
        return false;
    }
    if RESERVED_DOMAINS.contains(&domain_lower.as_str()) {
        return false;
    }
    if domain_lower.rsplit('.').next().is_some_and(|t| RESERVED_TLDS.contains(&t)) {
        return false;
    }
    let l = local.to_ascii_lowercase();
    if l.starts_with("noreply") || l.starts_with("no-reply") || l.starts_with("donotreply") || l.starts_with("do-not-reply") {
        return false;
    }
    // 1 文字 local + 1 文字ラベルの組は実アドレスとして事実上あり得ない
    !(local.chars().count() < 3 && domain_lower.split('.').next().unwrap_or("").chars().count() < 3)
}

// Unicode の Nd は 10 個連続の block で、先頭が 0。Python の int() が全角数字なども読むので、
// CIDR の長さを Python と同じ値にするため各 block の 0 の code point を持つ (Unicode 16、昇順)
const ND_ZEROS: [u32; 76] = [
    0x30, 0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66, 0xDE6, 0xE50, 0xED0, 0xF20, 0x1040,
    0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0,
    0xAA50, 0xABF0, 0xFF10, 0x104A0, 0x10D30, 0x10D40, 0x11066, 0x110F0, 0x11136, 0x111D0, 0x112F0, 0x11450, 0x114D0, 0x11650,
    0x116C0, 0x116D0, 0x116DA, 0x11730, 0x118E0, 0x11950, 0x11BF0, 0x11C50, 0x11D50, 0x11DA0, 0x11F50, 0x16130, 0x16A60, 0x16AC0,
    0x16B50, 0x16D70, 0x1CCF0, 0x1D7CE, 0x1D7D8, 0x1D7E2, 0x1D7EC, 0x1D7F6, 0x1E140, 0x1E2F0, 0x1E4F0, 0x1E5F1, 0x1E950, 0x1FBF0,
];

// `\d` に当たった文字 (Nd) の 0-9 の値。呼び出しは `\d` に当たった文字に限る
fn nd_value(c: char) -> u32 {
    let cp = c as u32;
    cp - ND_ZEROS[ND_ZEROS.partition_point(|&z| z <= cp) - 1]
}

fn group<'t>(c: &Captures<'t, str>, i: usize) -> &'t str {
    c.get(i).map_or("", |m| m.as_str())
}

fn plain() -> Check {
    Box::new(|c, _| Some(group(c, 0).to_string()))
}

fn assigned(group_no: usize, threshold: f64) -> Check {
    Box::new(move |c, _| {
        let value = group(c, group_no);
        (!(is_weak_value(value) || shannon_entropy(value) < threshold)).then(|| value.to_string())
    })
}

fn rule(class: &'static str, id: &'static str, pattern: &str, check: Check) -> Rule {
    Rule { class, id, re: re(pattern), check }
}

/// 行内の `rest` の直後が `/<prefix>` の CIDR 表記か。範囲の記述は実環境の識別子ではない。
/// 単一ホストを指す /32 (IPv4) と /128 (IPv6) だけは通常のアドレスとして扱う。
/// これが無いと、予約レンジを説明する文書やコード内の 2000::/3 のような記述を自分自身の gate が撃つ。
fn is_network_notation(cidr: &Regex, rest: &str, full_bits: u32) -> bool {
    let Ok(Some(m)) = cidr.captures(rest) else { return false };
    m.get(1).is_some_and(|d| d.as_str().chars().fold(0u32, |a, c| a * 10 + nd_value(c)) < full_bits)
}

fn rest_of<'a>(c: &Captures<str>, line: &'a str) -> &'a str {
    &line[c.get(0).map_or(line.len(), |m| m.end())..]
}

pub fn build(entropy: &Entropy) -> Vec<Rule> {
    let ent = |key: &str, default: f64| entropy.get(key, default);
    let cidr = Rc::new(re(r"^/(\d{1,3})(?![0-9])"));
    let (cidr4, cidr6) = (cidr.clone(), cidr);

    let mac_check: Check = Box::new(|c, _| check_mac(group(c, 1)).then(|| group(c, 1).to_string()));
    // 数字だけの 6 グループは日時 (yy-mm-dd-hh-mm-ss) や連番と区別できない。
    // 実 MAC が a-f を 1 文字も含まない確率は (10/16)^12 で 0.4% 程度なので、そこは取りこぼす
    let mac_hyphen_check: Check = Box::new(|c, _| {
        let v = group(c, 1);
        (v.chars().any(|x| matches!(x, 'A'..='F' | 'a'..='f')) && check_mac(v)).then(|| v.to_string())
    });
    let ipv4_check: Check = Box::new(move |c, line| {
        (!is_network_notation(&cidr4, rest_of(c, line), 32) && check_ipv4(group(c, 1))).then(|| group(c, 1).to_string())
    });
    let ipv6_check: Check = Box::new(move |c, line| {
        (!is_network_notation(&cidr6, rest_of(c, line), 128) && check_ipv6(group(c, 1))).then(|| group(c, 1).to_string())
    });
    let email_check: Check = Box::new(|c, line| {
        let (local, domain) = (group(c, 1), group(c, 2));
        check_email(local, domain, rest_of(c, line)).then(|| format!("{local}@{domain}"))
    });
    let arn_account: Check = Box::new(|c, _| check_aws_account(group(c, 1)).then(|| group(c, 1).to_string()));
    let sas: Check = Box::new(|c, line| {
        // `[?&]s[vper]=`
        let has_sig_param = line.as_bytes().windows(4).any(|w| {
            matches!(w[0], b'?' | b'&') && w[1] == b's' && matches!(w[2], b'v' | b'p' | b'e' | b'r') && w[3] == b'='
        });
        (has_sig_param || line.contains("core.windows.net")).then(|| group(c, 1).to_string())
    });

    vec![
        // --- net-* ---
        rule(
            "net-mac",
            "mac",
            r"(?<![0-9A-Fa-f:])((?:[0-9A-Fa-f]{1,2}:){5}[0-9A-Fa-f]{1,2})(?![0-9A-Fa-f:])",
            mac_check,
        ),
        rule(
            "net-mac",
            "mac",
            r"(?<![0-9A-Fa-f:-])((?:[0-9A-Fa-f]{2}-){5}[0-9A-Fa-f]{2})(?![0-9A-Fa-f:-])",
            mac_hyphen_check,
        ),
        rule("net-ipv4", "ipv4", r"(?<![0-9A-Za-z.\-])((?:\d{1,3}\.){3}\d{1,3})(?![0-9A-Za-z.\-])", ipv4_check),
        rule(
            "net-ipv6",
            "ipv6",
            r"(?<![0-9A-Za-z:.])((?:[0-9A-Fa-f]{0,4}:){2,7}[0-9A-Fa-f]{0,4}(?:%[0-9A-Za-z]+)?)(?![0-9A-Za-z:.])",
            ipv6_check,
        ),
        // --- email ---
        rule(
            "email",
            "email",
            r"(?<![A-Za-z0-9._%+-])([A-Za-z0-9._%+-]+)@([A-Za-z0-9.-]+\.[A-Za-z]{2,})",
            email_check,
        ),
        // --- credential (高信頼プレフィックス層) ---
        rule("credential", "aws-akid", r"(?<![A-Z0-9])(?:AKIA|ASIA|ABIA|ACCA)[0-9A-Z]{16}(?![A-Z0-9])", plain()),
        rule("credential", "gh-token", r"(?<![A-Za-z0-9_])gh[pousr]_[A-Za-z0-9]{36,255}", plain()),
        rule("credential", "gh-pat", r"github_pat_[0-9a-zA-Z_]{82}", plain()),
        rule("credential", "slack", r"xox[abprs]-[0-9A-Za-z-]{10,}", plain()),
        rule("credential", "llm-key", r"(?<![A-Za-z0-9_-])sk-(?:ant-)?[A-Za-z0-9_-]{20,}", plain()),
        rule("credential", "stripe", r"(?<![A-Za-z0-9_])(?:sk|rk)_(?:live|test)_[0-9a-zA-Z]{20,}", plain()),
        rule("credential", "pem", r"-----BEGIN(?: [A-Z]+)* PRIVATE KEY-----", plain()),
        rule(
            "credential",
            "jwt",
            r"(?<![A-Za-z0-9_-])eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
            plain(),
        ),
        rule("credential", "npm", r"(?<![A-Za-z0-9_])npm_[A-Za-z0-9]{36}(?![A-Za-z0-9])", plain()),
        // Dynatrace: prefix . 公開部 24 桁 . 秘密部 64 桁。c01 / s01 以外の prefix (s02 / s06 / s16 等) も同じ形に含める。
        // 桁数の同形は公式 docs の s01 でだけ確認している
        rule(
            "credential",
            "dynatrace",
            r"(?<![A-Za-z0-9_])dt0[cs]\d{2}\.[A-Z0-9]{24}\.[A-Z0-9]{64}(?![A-Za-z0-9])",
            plain(),
        ),
        // Datadog: API key は 32 桁 / application key は 40 桁の小文字 16 進。キー名で縛り、桁数が合わない値は撃たない
        rule(
            "credential",
            "datadog-api-key",
            r#"(?<![A-Za-z0-9_])DD_API_KEY["']?[<S>]*[:=][<S>]*["']?([0-9a-f]{32})(?![0-9a-f])"#,
            assigned(1, ent("datadog", 3.0)),
        ),
        rule(
            "credential",
            "datadog-app-key",
            concat!(
                r#"(?<![A-Za-z0-9_])(?:DD_APP_KEY|(?i:appl[i<X>]cat[i<X>]on[_-]?key))["']?[<S>]*[:=][<S>]*["']?"#,
                r"([0-9a-f]{40})(?![0-9a-f])"
            ),
            assigned(1, ent("datadog", 3.0)),
        ),
        // Splunk HEC: `Authorization: Splunk <GUID>`。GUID だけでは tenant id 等と区別できないので header 形に限る
        rule(
            "credential",
            "splunk-hec",
            concat!(
                r#"(?i:author[i<X>]zat[i<X>]on)["']?[<S>]*[:=][<S>]*["']?Splunk[<S>]+"#,
                r"([0-9A-Fa-f]{8}-(?:[0-9A-Fa-f]{4}-){3}[0-9A-Fa-f]{12})(?![0-9A-Fa-f])"
            ),
            assigned(1, ent("splunk-hec", 2.5)),
        ),
        // --- credential (汎用代入層。クォート必須 + エントロピーで締める) ---
        rule(
            "credential",
            "generic",
            concat!(
                r"(?i)(?<![A-Za-z0-9_<X>])(?:pass|passwd|password|secret|token|ap[i<X>][_-]?key|ap[i<X>]key|",
                r"access[_-]?key|cl[i<X>]ent[_-]?secret|auth[_-]?token)",
                r#"["']?[<S>]*[:=][<S>]*["']([A-Za-z0-9+/=_.\-<X>]{16,})["']"#
            ),
            assigned(1, ent("generic", 3.0)),
        ),
        // --- cloud-credential: GCP ---
        rule(
            "cloud-credential",
            "gcp-sa-json",
            r#""type"[<S>]*:[<S>]*"service_account"|"private_key_id"[<S>]*:[<S>]*"[0-9a-f]{40}""#,
            plain(),
        ),
        rule("cloud-credential", "gcp-api-key", r"(?<![A-Za-z0-9_-])AIza[0-9A-Za-z_-]{35}(?![A-Za-z0-9_-])", plain()),
        rule("cloud-credential", "gcp-oauth-sec", r"GOCSPX-[A-Za-z0-9_-]{28}", plain()),
        // OAuth access token。`ya29.c.` のように内部に `.` を持つ形があるので文字クラスに含める
        rule("cloud-credential", "gcp-oauth-token", r"(?<![A-Za-z0-9_-])ya29\.[A-Za-z0-9._-]{20,}", plain()),
        rule("cloud-credential", "gcp-refresh", r"(?<![A-Za-z0-9/])1//0[A-Za-z0-9_-]{30,}", plain()),
        // --- cloud-credential: AWS ---
        rule(
            "cloud-credential",
            "aws-secret-key",
            concat!(
                r"(?i)aws[_-]?(?:secret|pr[i<X>]vate)[_-]?(?:access[_-]?)?key",
                r#"["']?[<S>]*[:=][<S>]*["']?([A-Za-z0-9/+=<X>]{40})(?![A-Za-z0-9/+=<X>])"#
            ),
            assigned(1, ent("aws-secret-key", 4.0)),
        ),
        // STS / IAM の XML レスポンス。値は別行になり得るが行単位走査なので同一行の形だけ拾う。
        // JSON の "SecretAccessKey" は対象外 (同じ応答の AccessKeyId を aws-akid が止めるため、指示も XML のみ)
        rule(
            "cloud-credential",
            "aws-secret-xml",
            r"<SecretAccessKey>([A-Za-z0-9/+=]{40})(?![A-Za-z0-9/+=])",
            assigned(1, ent("aws-secret-key", 4.0)),
        ),
        rule(
            "cloud-credential",
            "aws-session-token",
            concat!(
                r"(?i)aws[_-]?sess[i<X>]on[_-]?token",
                r#"["']?[<S>]*[:=][<S>]*["']?([A-Za-z0-9/+=<X>]{100,})"#
            ),
            assigned(1, ent("aws-session-token", 3.5)),
        ),
        rule("cloud-credential", "aws-arn-acct", r"arn:aws[a-z-]*:[a-z0-9-]*:[a-z0-9-]*:([0-9]{12}):", arn_account),
        // --- cloud-credential: Azure ---
        rule("cloud-credential", "azure-storage", r"AccountKey=[A-Za-z0-9+/=]{80,}", plain()),
        rule("cloud-credential", "azure-sas", r"[?&]sig=([A-Za-z0-9%+/=]{40,})", sas),
        rule(
            "cloud-credential",
            "azure-client-secret",
            concat!(
                r"(?i)(?:azure|arm)[_-]?cl[i<X>]ent[_-]?secret",
                r#"["']?[<S>]*[:=][<S>]*["']?([A-Za-z0-9._~<X>-]{32,44})"#
            ),
            assigned(1, ent("azure-client-secret", 3.5)),
        ),
        rule(
            "cloud-credential",
            "azure-adv2",
            r"(?<![A-Za-z0-9_~.-])[A-Za-z0-9_~.-]{3}8Q~[A-Za-z0-9_~.-]{34}(?![A-Za-z0-9_~.-])",
            plain(),
        ),
        // --- db-credential ---
        rule(
            "db-credential",
            "url",
            concat!(
                r"(?i)(?<![A-Za-z0-9+.\-<X>])(?:postgres|postgresql|mysql|mar[i<X>]adb|mongodb\+srv|mongodb|",
                r"red[i<X>]ss|red[i<X>]s|amqps|amqp|cl[i<X>]ckhouse|cockroachdb|mssql|sqlserver)://",
                r#"[^:@/<S>"']{1,64}:([^@/<S>"']{6,128})@"#
            ),
            assigned(1, ent("db", 2.0)),
        ),
        rule(
            "db-credential",
            "jdbc",
            r#"(?i)jdbc:[a-z0-9<X>]+://[^<S>"']*[?&;](?:password|pwd)=([^<S>&;"']{6,})"#,
            assigned(1, ent("db", 2.0)),
        ),
        rule(
            "db-credential",
            "env",
            concat!(
                r"(?<![A-Za-z0-9_])(?:PGPASSWORD|MYSQL_PWD|MYSQL_ROOT_PASSWORD|MARIADB_PASSWORD|",
                r"POSTGRES_PASSWORD|MONGO_INITDB_ROOT_PASSWORD|REDIS_PASSWORD|DB_PASSWORD|",
                r#"DATABASE_PASSWORD|MSSQL_SA_PASSWORD)[<S>]*=[<S>]*["']?([^<S>"']{6,})"#
            ),
            assigned(1, ent("db", 2.0)),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Vec<Rule> {
        build(&Entropy::from_policy(None).unwrap())
    }

    #[test]
    fn ルール表は_python_実装と同じ_35_本で_id_の並びが一致する() {
        let ids: Vec<_> = rules().iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), 35);
        assert_eq!(&ids[..5], ["mac", "mac", "ipv4", "ipv6", "email"]);
        assert_eq!(&ids[13..18], ["npm", "dynatrace", "datadog-api-key", "datadog-app-key", "splunk-hec"]);
        assert_eq!(&ids[32..], ["url", "jdbc", "env"]);
    }

    #[test]
    fn nd_value_は全角やインド系の数字も_0_から_9_に直す() {
        assert_eq!(nd_value('7'), 7);
        assert_eq!(nd_value('\u{FF13}'), 3);
        assert_eq!(nd_value('\u{0669}'), 9);
        assert_eq!(nd_value('\u{096F}'), 9);
        assert_eq!(nd_value('\u{1D7FF}'), 9);
    }

    #[test]
    fn is_global_は_ipaddress_の予約レンジと例外に従う() {
        let v4 = |s: &str| is_global4(u32::from(s.parse::<Ipv4Addr>().unwrap()));
        assert!(v4("8.8.8.8") && v4("192.0.0.9") && v4("192.0.0.10"));
        assert!(!v4("192.0.0.8") && !v4("192.0.0.11") && !v4("100.64.0.1") && !v4("10.0.0.1") && !v4("255.255.255.255"));
        assert!(check_ipv6("2606:4700:4700::1111") && check_ipv6("2001:3::1") && check_ipv6("2001:1::1"));
        assert!(!check_ipv6("2001:db8::1") && !check_ipv6("2001:1::3") && !check_ipv6("3fff::1") && !check_ipv6("fe80::1%eth0"));
    }

    #[test]
    fn mac_と_aws_account_と_email_の判定() {
        assert!(check_mac("a4:83:e7:11:22:33") && check_mac("A4-83-E7-11-22-33"));
        assert!(!check_mac("01:00:5e:00:00:01") && !check_mac("00:00:5e:00:53:01") && !check_mac("0:0:0:0:0:0"));
        assert!(!check_aws_account("123456789012") && !check_aws_account("111122223333") && check_aws_account("493827160529"));
        assert!(!check_email("git", "GitHub.com", "") && !check_email("a", "b.io", "") && check_email("alice", "corp.io", ""));
        assert!(!check_email("alice", "corp.io", ":path") && check_email("alice", "corp.io", ": x"));
    }
}
