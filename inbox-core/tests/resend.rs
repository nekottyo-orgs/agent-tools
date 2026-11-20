// 消す前の bash の resend_step を全組で回した結果 (resend.decisions.tsv) と、resend の出力が全行で一致すること
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn resend_全行が_bash_の出力と一致する() {
    let tsv = include_str!("resend.decisions.tsv");
    let rows: Vec<Vec<&str>> = tsv
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split('\t').collect())
        .collect();
    assert_eq!(rows.len(), 4608);
    let mut input = String::new();
    for r in &rows {
        assert_eq!(r.len(), 12);
        input.push_str(&format!("resend {}\n", r[..11].join(" ")));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_inbox-core"))
        .arg("batch")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let got = String::from_utf8(out.stdout).unwrap();
    let got: Vec<&str> = got.lines().collect();
    assert_eq!(got.len(), rows.len());
    for (i, r) in rows.iter().enumerate() {
        assert_eq!(got[i], r[11], "行 {}: {}", i + 2, r[..11].join(" "));
    }
}
