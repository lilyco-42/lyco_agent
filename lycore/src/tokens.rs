//! seg_zh tokenizer — lyv.tokens() 的 Rust 移植 (字节级对齐目标)
//!
//! Python 原型 (lyv.py tokens()):
//!   1. ASCII 词: [A-Za-z][A-Za-z0-9_-]* 提取, 小写化
//!   2. CJK: 过滤出所有汉字, 相邻 bigram
//!
//! 对齐协议: 同一输入, Rust 与 Python 产出的 token 列表完全一致 (顺序+内容)。
//! 索引侧 (build) 与查询侧 (query) 必须用同一实现, 否则 FTS 召回漂移。

/// Python ASCII_TOK = r"[A-Za-z][A-Za-z0-9_\-]*" 的等价提取
/// findall 语义: 从头扫, 每个匹配是一个词 — 数字开头段不是词首, 但 "123" 中
/// 的字符若紧跟字母则并入 (如 "hello123" → "hello123"; "123abc" → "abc")
fn ascii_tokens(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphabetic() {
            let start = i;
            let mut end = i + 1;
            while end < chars.len()
                && (chars[end].is_ascii_alphanumeric() || chars[end] == '_' || chars[end] == '-')
            {
                end += 1;
            }
            let word: String = chars[start..end].iter().collect();
            out.push(word.to_ascii_lowercase());
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// CJK bigram: Python 的 zh = re.sub(r"[^一-鿿]", " ", text) — 非汉字变空格,
/// 然后取相邻滑窗 (含空格! 如 " 建"/"先 "/"目目")。Rust 必须复刻此行为:
/// 1. 非汉字字符 → 空格
/// 2. 对替换后的字符串取 len-1 个滑窗 bigram, 过滤 strip 后为空的
fn cjk_bigrams(text: &str) -> Vec<String> {
    let spaced: Vec<char> = text
        .chars()
        .map(|c| {
            if ('\u{4e00}'..='\u{9fff}').contains(&c) {
                c
            } else {
                ' '
            }
        })
        .collect();
    if spaced.len() < 2 {
        return Vec::new();
    }
    spaced
        .windows(2)
        .map(|w| w.iter().collect::<String>())
        .filter(|s| !s.trim().is_empty()) // Python: 保留原始切片, 仅过滤 strip 后为空的
        .collect()
}

/// tokens(text): ASCII 词 + CJK bigram (与 lyv.tokens() 输出顺序一致: 先全部 ASCII 词, 再 bigram)
pub fn tokens(text: &str) -> Vec<String> {
    let mut t = ascii_tokens(text);
    t.extend(cjk_bigrams(text));
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    // Python 参考输出 (tools/token_ref.py 实测, 逐字节对齐)
    #[test]
    fn python_alignment_cases() {
        // 含跨空格 bigram: " 建" "先 " 等 (Python zh 替换空格后的滑窗产物, FTS 索引含这些)
        assert_eq!(
            tokens("首先 cargo new hello_world 建立项目"),
            vec![
                "cargo".to_string(),
                "new".to_string(),
                "hello_world".to_string(),
                "首先".to_string(),
                "先 ".to_string(),
                " 建".to_string(),
                "建立".to_string(),
                "立项".to_string(),
                "项目".to_string()
            ]
        );
        assert_eq!(
            tokens("cargo run"),
            vec!["cargo".to_string(), "run".to_string()]
        );
        // '目目' — 标点替换为空格后的跨字 bigram
        assert_eq!(
            tokens("进入项目目录"),
            vec![
                "进入".to_string(),
                "入项".to_string(),
                "项目".to_string(),
                "目目".to_string(),
                "目录".to_string()
            ]
        );
        assert_eq!(
            tokens("hello world 123"),
            vec!["hello".to_string(), "world".to_string()]
        );
        assert_eq!(tokens(""), Vec::<String>::new());
        assert_eq!(tokens("!!! ###"), Vec::<String>::new());
        // 标点+句号: "最后, Cargo Run 运行程序输出 Hello World。"
        assert_eq!(
            tokens("最后, Cargo Run 运行程序输出 Hello World。"),
            vec![
                "cargo".to_string(),
                "run".to_string(),
                "hello".to_string(),
                "world".to_string(),
                "最后".to_string(),
                "后 ".to_string(),
                " 运".to_string(),
                "运行".to_string(),
                "行程".to_string(),
                "程序".to_string(),
                "序输".to_string(),
                "输出".to_string(),
                "出 ".to_string()
            ]
        );
    }
}
