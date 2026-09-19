//! 与 token_ref.py 的 JSON 输出做全量对齐 (金标准来自 Python 运行时)
use lycore::tokens::tokens;

#[test]
#[ignore = "需要 Python 生成的参考文件"]
fn align_with_python_reference() {
    let Ok(raw) = std::fs::read_to_string("../../tools/token_ref_expected.json") else {
        eprintln!("skip: 参考文件不存在");
        return;
    };
    let cases: std::collections::HashMap<String, Vec<String>> = serde_json::from_str(&raw).unwrap();
    let mut mismatches = 0;
    for (input, expected) in &cases {
        let got = tokens(input);
        if &got != expected {
            mismatches += 1;
            eprintln!("MISMATCH {input:?}\n  py: {expected:?}\n  rs: {got:?}");
        }
    }
    assert_eq!(mismatches, 0, "{} 个用例不一致", mismatches);
}
