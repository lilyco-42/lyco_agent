//! agent loop 编排 — lyco_agent_loop.py run() 的 Rust 移植
//!
//! 架构: ModelBackend trait 抽象模型 IO, 编排器串联 执行器+学习队列。
//!   - ScriptedBackend: 回放脚本 (测试用, 不需要真模型)
//!   - 未来: LlamaCppBackend (llama.cpp server HTTP) / BitNetBackend (端侧)
//!
//! 多轮协议 (与 Python 版一致):
//!   model → tool_call → executor → tool result 回填 → model → ... (≤max_rounds)
//!   模型不再发 tool_call → 最终回答; NO_HIT/未知工具 → 学习队列 + 诚实降级

use crate::executor::{parse_call, Executor, ToolResult};
use serde::Serialize;

/// 模型 IO 抽象: 输入消息历史, 输出模型生成文本
pub trait ModelBackend {
    fn generate(&mut self, messages: &[Message]) -> anyhow::Result<String>;
}

/// 对话消息 (role: system/user/assistant/tool)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self { role: role.into(), content: content.into() }
    }
}

/// 编排结果
#[derive(Debug, Serialize)]
pub struct Turn {
    pub rounds: usize,
    pub answer: String,
    pub tool_calls: Vec<String>,
    pub learning_queue_used: bool,
}

pub struct Agent<'a> {
    pub executor: &'a Executor,
    pub max_rounds: usize,
}

impl<'a> Agent<'a> {
    pub fn new(executor: &'a Executor, max_rounds: usize) -> Self {
        Self { executor, max_rounds }
    }

    /// 主循环: 模型↔工具 多轮交互直到收束
    pub fn run(&self, backend: &mut dyn ModelBackend, question: &str) -> anyhow::Result<Turn> {
        let mut messages = vec![
            Message::new("system", "You are lyco. 操作类问题先用工具查询知识库."),
            Message::new("user", question),
        ];
        let mut tool_calls = Vec::new();
        let mut learning_queue_used = false;
        // 回填复读防护: 记录失败过的工具, 模型重复调用同名失败工具 = 卡死信号, 短路收束
        let mut failed_tools: Vec<String> = Vec::new();

        for round in 1..=self.max_rounds {
            let text = backend.generate(&messages)?;
            match parse_call(&text) {
                None => {
                    // 最终回答: 剔除工具残留 + JSON 复读转人话 (Python 版同款逻辑)
                    let mut answer = strip_tags(&text);
                    if answer.starts_with('{') && answer.contains("\"ok\"") {
                        if let Ok(r) = serde_json::from_str::<serde_json::Value>(&answer) {
                            answer = if r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                                format!(
                                    "{} 查询完成: {}",
                                    r.get("tool").and_then(|v| v.as_str()).unwrap_or("工具"),
                                    r.get("answer").and_then(|v| v.as_str()).unwrap_or("")
                                )
                            } else {
                                "抱歉，我还没学会这个操作，已加入学习队列。".to_string()
                            };
                        }
                    }
                    return Ok(Turn { rounds: round, answer, tool_calls, learning_queue_used });
                }
                Some((name, arguments)) => {
                    // 复读已失败的工具 → 不再空执行, 直接诚实收束 (省轮次, 避免 max_rounds 兜底)
                    if failed_tools.iter().any(|f| f == &name) {
                        return Ok(Turn {
                            rounds: round,
                            answer: format!(
                                "抱歉，{name} 工具查不到这个内容，我还没学会，已加入学习队列。"
                            ),
                            tool_calls,
                            learning_queue_used: true,
                        });
                    }
                    tool_calls.push(name.clone());
                    let result = self.executor.execute(&name, &arguments);
                    if !result.ok {
                        learning_queue_used = true;
                        failed_tools.push(name.clone());
                    }
                    log_result(&result);
                    messages.push(Message::new(
                        "assistant",
                        format!("<tool_call>\n{}\n</tool_call>", serde_json::to_string(&call_value(&name, &arguments))?),
                    ));
                    messages.push(Message::new(
                        "tool",
                        serde_json::to_string(&result)?,
                    ));
                }
            }
        }
        Ok(Turn {
            rounds: self.max_rounds,
            answer: "任务未在限定轮数内收束".to_string(),
            tool_calls,
            learning_queue_used,
        })
    }
}

fn call_value(name: &str, arguments: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "name": name, "arguments": arguments })
}

fn strip_tags(text: &str) -> String {
    let mut s = text.to_string();
    for tag in ["<|im_end|>", "<tool_call>", "</tool_call>", "<error>", "</error>"] {
        s = s.replace(tag, "");
    }
    // 从 im_end 后截断的语义由 replace 保留 (删掉标记后可能留尾部) — 与 Python 正则版近似
    s.trim().to_string()
}

fn log_result(r: &ToolResult) {
    let summary = if r.ok { "ok" } else { r.error.as_deref().unwrap_or("fail") };
    eprintln!("[agent] tool={} result={}", r.tool, summary);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::LearningQueue;
    use std::path::Path;

    /// 回放后端: 按预设脚本依次输出 (对应真实 GRPO 模型今天在 A10 上的输出)
    struct Scripted {
        steps: Vec<String>,
        i: usize,
    }
    impl Scripted {
        fn new(steps: Vec<&str>) -> Self {
            Self { steps: steps.iter().map(|s| s.to_string()).collect(), i: 0 }
        }
    }
    impl ModelBackend for Scripted {
        fn generate(&mut self, _messages: &[Message]) -> anyhow::Result<String> {
            let s = self.steps.get(self.i).cloned().unwrap_or_default();
            self.i += 1;
            Ok(s)
        }
    }

    fn executor_on_real_pack() -> Executor {
        Executor::open(Path::new("../smoke/pack_final")).expect("open pack")
    }

    #[test]
    #[ignore = "需要真实知识包"]
    fn multihop_call_then_final_answer() {
        let ex = executor_on_real_pack();
        let agent = Agent::new(&ex, 4);
        let mut backend = Scripted::new(vec![
            // round 1: 发起工具调用 (GRPO 模型真实输出样式)
            r#"<tool_call>
{"name": "lyv_knowledge", "arguments": {"query": "怎么运行项目", "pack": "/video/123456"}}
</tool_call>"#,
            // round 2: 收到 tool result 后收束
            "运行项目的操作在视频 15.0s-20.0s 有演示, 请查看对应切片。",
        ]);
        let turn = agent.run(&mut backend, "怎么运行项目").unwrap();
        assert_eq!(turn.rounds, 2);
        assert_eq!(turn.tool_calls, vec!["lyv_knowledge".to_string()]);
        assert!(!turn.learning_queue_used);
        assert!(turn.answer.contains("15.0s"));
    }

    #[test]
    #[ignore = "需要真实知识包"]
    fn no_hit_routes_to_learning_queue() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("index")).unwrap();
        std::fs::copy(
            "../smoke/pack_final/index/knowledge.sqlite",
            tmp.path().join("index/knowledge.sqlite"),
        )
        .unwrap();
        let ex = Executor::open(tmp.path()).unwrap();
        let agent = Agent::new(&ex, 4);
        let mut backend = Scripted::new(vec![
            r#"<tool_call>
{"name": "lyv_knowledge", "arguments": {"query": "怎么配置防火墙"}}
</tool_call>"#,
            // 模型复读 JSON → 编排器转人话
            r#"{"ok": false, "tool": "lyv_knowledge", "error": "NO_HIT"}"#,
        ]);
        let turn = agent.run(&mut backend, "怎么配置防火墙").unwrap();
        assert!(turn.learning_queue_used);
        assert_eq!(turn.answer, "抱歉，我还没学会这个操作，已加入学习队列。");
        let q = LearningQueue::open(tmp.path());
        assert_eq!(q.len(), 1);
    }

    #[test]
    #[ignore = "需要真实知识包"]
    fn max_rounds_guard() {
        let ex = executor_on_real_pack();
        let agent = Agent::new(&ex, 2);
        let mut backend = Scripted::new(vec![
            r#"<tool_call>
{"name": "lyv_knowledge", "arguments": {"query": "运行项目"}}
</tool_call>"#,
            r#"<tool_call>
{"name": "lyv_knowledge", "arguments": {"query": "运行项目"}}
</tool_call>"#,
        ]);
        let turn = agent.run(&mut backend, "怎么运行项目").unwrap();
        assert_eq!(turn.rounds, 2);
        assert!(turn.answer.contains("未在限定轮数内收束"));
    }

    /// 复读防护: 模型重复调用已失败的同名工具 → 立即诚实收束, 不空耗轮次
    #[test]
    #[ignore = "需要真实知识包"]
    fn repeat_failed_tool_short_circuits() {
        let ex = executor_on_real_pack();
        let agent = Agent::new(&ex, 4);
        // "怎么配置防火墙" 在 pack_final 中 NO_HIT → 第 1 轮失败入 failed_tools,
        // 第 2 轮模型复读同名工具被短路 (rounds=2, 而非耗尽 max_rounds=4)
        let repeat = r#"<tool_call>
{"name": "lyv_knowledge", "arguments": {"query": "怎么配置防火墙"}}
</tool_call>"#;
        let mut backend = Scripted::new(vec![repeat, repeat, "不应到达"]);
        let turn = agent.run(&mut backend, "怎么配置防火墙").unwrap();
        assert_eq!(turn.rounds, 2, "复读应在第 2 轮收束, 实得 {}", turn.rounds);
        assert!(turn.learning_queue_used);
        assert!(turn.answer.contains("还没学会"), "answer={}", turn.answer);
    }
}
