//! 集成测试: 用真实 pack_final (Python lyv 构建的 sqlite) 验证 Rust 检索一致性
//! 运行: cargo test -- --ignored (需要 smoke/pack_final 存在)

use lycore::pack::Pack;
use std::path::Path;

const PACK: &str = "../smoke/pack_final";

#[test]
#[ignore = "需要真实知识包 (smoke/pack_final)"]
fn lookup_hits_intent_dict() {
    if !Path::new(PACK).exists() {
        eprintln!("skip: pack_final 不存在");
        return;
    }
    let pack = Pack::open(Path::new(PACK)).expect("open pack");
    // Python 版同样命中 rust.project.run (t0=15.0, t1=20.0)
    let ev = pack.lookup("怎么运行项目").expect("query").expect("hit");
    assert_eq!(ev.intent, "rust.project.run");
    assert_eq!(ev.t0, 15.0);
    assert_eq!(ev.t1, 20.0);
    assert!(ev.text.contains("Cargo Run"));
    assert_eq!(ev.retrieval, "intent-dict:rust.project.run");
}

#[test]
#[ignore = "需要真实知识包"]
fn no_hit_returns_none() {
    if !Path::new(PACK).exists() {
        eprintln!("skip: pack_final 不存在");
        return;
    }
    let pack = Pack::open(Path::new(PACK)).expect("open pack");
    let ev = pack.lookup("怎么配置防火墙").expect("query");
    assert!(ev.is_none(), "不存在的操作应返回 None (学习队列语义)");
}
