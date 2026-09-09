//! executor 集成测试: 对真实 pack_final 验证工具执行全路径
use lycore::executor::{parse_call, Executor};

#[test]
#[ignore = "需要真实知识包 (smoke/pack_final)"]
fn hit_path_returns_evidence() {
    let ex = Executor::open(std::path::Path::new("../smoke/pack_final")).unwrap();
    let args = serde_json::json!({"query": "怎么运行项目", "pack": "幻觉路径"});
    let r = ex.execute("lyv_knowledge", &args);
    assert!(r.ok);
    assert!(r.answer.unwrap().contains("Cargo Run"));
    assert!(r.clip.unwrap().contains("15.0s"));
}

#[test]
#[ignore = "需要真实知识包"]
fn no_hit_pushes_learning_queue() {
    let tmp = tempfile::tempdir().unwrap();
    // 复制知识包到临时目录, 让学习队列写进 tempdir
    std::fs::create_dir_all(tmp.path().join("index")).unwrap();
    std::fs::copy(
        "../smoke/pack_final/index/knowledge.sqlite",
        tmp.path().join("index/knowledge.sqlite"),
    )
    .unwrap();
    let ex = Executor::open(tmp.path()).unwrap();
    let r = ex.execute("lyv_knowledge", &serde_json::json!({"query": "怎么配置防火墙"}));
    assert!(!r.ok);
    assert!(r.error.unwrap().contains("学习队列"));
    assert_eq!(ex.learning_queue().len(), 1, "NO_HIT 应入学习队列");
}

#[test]
fn parse_call_two_level() {
    // <tool_call> 包裹
    let a = parse_call("<tool_call>\n{\"name\": \"lyv_knowledge\", \"arguments\": {\"query\": \"cargo new\"}}\n</tool_call>");
    assert_eq!(a.as_ref().unwrap().0, "lyv_knowledge");
    // 裸 JSON
    let b = parse_call("前置文字 {\"name\": \"vnn_identify\", \"arguments\": {\"image\": \"x.png\"}} 后缀");
    assert_eq!(b.as_ref().unwrap().0, "vnn_identify");
    // 非 JSON
    assert!(parse_call("你好呀").is_none());
}

#[test]
#[ignore = "需要真实知识包"]
fn unknown_tool_fails_honestly() {
    let ex = Executor::open(std::path::Path::new("../smoke/pack_final")).unwrap();
    let r = ex.execute("weather_query", &serde_json::json!({}));
    assert!(!r.ok);
    assert!(r.error.unwrap().contains("未知工具"));
}
