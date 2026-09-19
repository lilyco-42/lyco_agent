//! executor 集成测试: 对真实 pack_final 验证工具执行全路径
use lycore::executor::{parse_call, Executor};

/// 防漂移守卫 (非 ignored, CI 必跑): 权威 schema 的工具集是执行器认识的集合。
/// 本会话反复踩 "schema 与实现不一致" (CHAT_TOOLS 停在 2 工具、resolve 默认 v2
/// 使升级休眠), 此测试把这类漂移变成硬失败。新增工具须同时出现在三处:
/// llamacpp::CHAT_TOOLS、executor::execute 的 match、本期望列表。
#[test]
fn chat_tools_matches_executor_dispatch() {
    let tools = lycore::llamacpp::chat_tools();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    let expected = [
        "lyv_knowledge",
        "vnn_identify",
        "html_gen",
        "html_render_video",
        "rembg_remove",
        "llm_generate",
        "video_info",
        "shell_exec",
        "file_write",
        "schedule",
    ];
    assert_eq!(
        names, expected,
        "schema 漂移: 导出工具集 ≠ 期望 (与 executor match 对齐)"
    );
    // 每个工具都得有参数 schema (OpenAI 要求), 且 name 唯一
    assert_eq!(
        names.iter().collect::<std::collections::HashSet<_>>().len(),
        expected.len()
    );
    for t in tools.as_array().unwrap() {
        assert!(
            t["function"]["parameters"]["type"] == "object",
            "缺 parameters"
        );
    }

    // 真正的三方一致性: 每个 schema 工具都必须在 executor 的 match 分支里出现。
    // (读源码而非调 execute(): 空参调用会触发 ffmpeg/sqlite 副作用并污染学习队列)
    let exec_src = include_str!("../src/executor.rs");
    for n in expected {
        assert!(
            exec_src.contains(&format!("\"{n}\" =>")),
            "executor.rs 缺少 {n} 分发分支 (schema 声明了但执行器不认)"
        );
    }
}

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
    let r = ex.execute(
        "lyv_knowledge",
        &serde_json::json!({"query": "怎么配置防火墙"}),
    );
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
    let b = parse_call(
        "前置文字 {\"name\": \"vnn_identify\", \"arguments\": {\"image\": \"x.png\"}} 后缀",
    );
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
