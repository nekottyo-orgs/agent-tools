// secret-scan のルール表。骨組みだけで、ルールの移植は別 PR で行う。
//
// ルール表は Python 実装 build_rules と同じ順序で並べる (finditer の報告順が findings の順序になる)。
//
// 正規表現を移すときの注意 (fancy-regex は Python の re と Unicode の扱いが違う):
//   - `\s`: Python は str.isspace() で `\x1c`-`\x1f` を含むが、Rust の `\s` は含まない。
//     `\s` は `[\s\x1c-\x1f]` に置き換える
//   - `\d`: どちらも既定で Unicode の Nd 全般に当たるが、`(?-u)` や `(?a)` を付けると ASCII に狭まる。
//     付けずにそのまま移し、全角数字を入れた差分検証で一致を確かめる
//   - `(?i)`: 大文字小文字の畳み込みの対象文字が Python と Rust で少し違う (U+017F や U+212A など)。
//     ASCII だけの語に `(?i)` を掛けているルールは、差分検証で確かめる
//   - Python の `(?i:...)` スコープ付きフラグは fancy-regex でも書ける。大文字固定の正規表現に
//     `(?i)` を掛けてはいけない (ENV_NAME_RE)

use crate::secret_scan::Entropy;
use fancy_regex::{Captures, Regex};

pub struct Rule {
    pub class: &'static str,
    pub id: &'static str,
    pub re: Regex,
    /// 検出とみなすなら報告値を返す。見送るなら None。第 2 引数は行全体 (Python の `m.string`)。
    pub check: Box<dyn Fn(&Captures<str>, &str) -> Option<String>>,
}

pub fn build(_entropy: &Entropy) -> Vec<Rule> {
    Vec::new()
}
