//! LlamaCppBackend — 对接 llama.cpp server (/v1/chat/completions, OpenAI 兼容)
//!
//! 端侧部署形态: llama-server 跑量化 GGUF (Qwen3-0.6B Q4 ~0.5GB),
//! lycore 通过 HTTP 调用 — ModelBackend trait 的第一个真实实现。
//!
//! 聊天模板: llama.cpp server 端应用 chat template (含 Qwen3 的 tool_call 格式),
//! Rust 侧只送 OpenAI messages + tools, 不在客户端拼模板 — 避免双端模板漂移。
//!
//! 启动参考 (A10/本机):
//!   llama-server -m qwen3-0.6b-q4.gguf --port 8081 --jinja
//!   (--jinja 启用 server 端 tool template, Qwen3 tool_call 必需)

use crate::agent::{Message, ModelBackend};
use serde_json::json;

const CHAT_TOOLS: &str = r#"[
  {"type":"function","function":{"name":"lyv_knowledge","description":"查询视频知识库: 问怎么做某操作, 返回带时间戳的视频切片+关键帧+OCR验证文字","parameters":{"type":"object","properties":{"query":{"type":"string","description":"想学的操作"}},"required":["query"]}}},
  {"type":"function","function":{"name":"vnn_identify","description":"OCR失败时启用内部识图神经网络对图片打分描述","parameters":{"type":"object","properties":{"image":{"type":"string","description":"图片路径"}},"required":["image"]}}}
]"#;

pub struct LlamaCppBackend {
    client: reqwest::blocking::Client,
    base_url: String,
    model: String,
    /// 非 thinking 模式 (Qwen3: 工具调用场景必须关, 否则 token 耗在 <think>)
    pub enable_thinking: bool,
}

impl LlamaCppBackend {
    pub fn new(base_url: &str, model: &str) -> Self {
        Self {
            client: reqwest::blocking::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            enable_thinking: false,
        }
    }

    /// 健康检查 (llama.cpp /health)
    pub fn health(&self) -> anyhow::Result<()> {
        let url = format!("{}/health", self.base_url);
        let status = self.client.get(&url).send()?.status();
        anyhow::ensure!(status.is_success(), "llama-server unhealthy: {status}");
        Ok(())
    }
}

impl ModelBackend for LlamaCppBackend {
    fn generate(&mut self, messages: &[Message]) -> anyhow::Result<String> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect();
        let body = json!({
            "model": self.model,
            "messages": msgs,
            "tools": serde_json::from_str::<serde_json::Value>(CHAT_TOOLS)?,
            "temperature": 0.7,
            "top_p": 0.8,
            "top_k": 20,
            "max_tokens": 300,
            // Qwen3 非思考模式: llama.cpp 用 chat_template_kwargs 传递
            "chat_template_kwargs": {"enable_thinking": self.enable_thinking},
        });
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()?
            .error_for_status()?;
        let v: serde_json::Value = resp.json()?;
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        // tool_call 可能落在 tool_calls 字段 (server 端解析) — 拼回文本格式给 parse_call。
        // 注意: server 的 content 有时已含模板输出的 <tool_call> 文本 → 此时不再拼接 (防双重)
        let mut text = text;
        if let Some(calls) = v["choices"][0]["message"]["tool_calls"].as_array() {
            if !text.contains("<tool_call>") {
                for c in calls {
                    let name = c["function"]["name"].as_str().unwrap_or("");
                    // arguments 可能是字符串 (未解析 JSON) — 规范化为 JSON 对象字面量
                    let args_val = match c["function"]["arguments"] {
                        serde_json::Value::String(ref s) => {
                            serde_json::from_str::<serde_json::Value>(s)
                                .unwrap_or(serde_json::json!({}))
                        }
                        ref v => v.clone(),
                    };
                    text.push_str(&format!(
                        "\n<tool_call>\n{}\n</tool_call>",
                        serde_json::json!({"name": name, "arguments": args_val})
                    ));
                }
            }
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "需要 llama-server 运行中 (localhost:8081)"]
    fn health_and_generate() {
        let mut b = LlamaCppBackend::new("http://localhost:8081", "qwen3-0.6b");
        b.health().expect("server 未启动");
        let messages = vec![
            Message::new("system", "You are lyco."),
            Message::new("user", "怎么新建 rust 项目"),
        ];
        let out = b.generate(&messages).expect("generate");
        assert!(!out.is_empty());
    }
}
