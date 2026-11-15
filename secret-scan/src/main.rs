// 目的: `git diff --unified=0` 形の diff を stdin で受け、追加行から実環境の識別子と credential を検出して
//       JSON で返す単体の binary。検出エンジンは secret_scan.rs、ルール表は secret_rules.rs。
//
// 安全設計 (why):
//   - 呼び出し側 (commit の gate) は終了コード 2 を fail-close として扱う。panic の既定の 101 は意味が決まっていないので、
//     panic も引数の不正も 2 に揃える
//   - subcommand を持たない。引数なしと未知の引数は exit 2 になり、「何も言わずに exit 0 と空の出力」を返す経路を作らない
//     (古い bash-gates は知らない subcommand にそう返していて、呼び出し側が検出 0 件と取り違えうる状態だった)
//
// 使い方: git diff --cached --unified=0 | secret-scan --policy <path> --repo owner/name
//
// 終了コード: 0 = 走査完了 (検出の有無は stdout の JSON)。2 = 引数不正・policy 読み込み失敗・stdin の読み取り失敗

mod secret_rules;
mod secret_scan;

fn main() {
    let args: Result<Vec<String>, _> = std::env::args_os().skip(1).map(|a| a.into_string()).collect();
    let code = match args {
        Ok(args) => std::panic::catch_unwind(|| secret_scan::run(args)).unwrap_or(2),
        Err(_) => {
            eprintln!("secret-scan: argument is not valid UTF-8");
            2
        }
    };
    std::process::exit(code);
}
