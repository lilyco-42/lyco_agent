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

// 与训练 schema (tools/qwen_grpo_v2.py TOOLS) 严格一致 — 运行时/训练时描述漂移会劣化 FC 决策
const CHAT_TOOLS: &str = r#"[{"type":"function","function":{"name":"lyv_knowledge","description":"查询【视频演示过的具体操作/命令步骤】: 用户问某软件/开发工具怎么用、某命令怎么敲、某操作在教程视频里怎么做。纯开放问答/创作/闲聊不属于此类。","parameters":{"type":"object","properties":{"query":{"type":"string","description":"想学的操作"},"pack":{"type":"string","description":"知识包路径"}},"required":["query"]}}},{"type":"function","function":{"name":"vnn_identify","description":"识别图片内容: 对截图/画面分类 (终端/GUI/自然/文档)","parameters":{"type":"object","properties":{"image":{"type":"string","description":"图片路径"}},"required":["image"]}}},{"type":"function","function":{"name":"html_gen","description":"生成 HTML 页面文件: 给页面需求描述, 调用大模型生成完整单文件 HTML 并保存","parameters":{"type":"object","properties":{"prompt":{"type":"string","description":"页面需求描述"},"output":{"type":"string","description":"输出 html 文件路径"}},"required":["prompt","output"]}}},{"type":"function","function":{"name":"html_render_video","description":"把 HTML 页面转成视频: headless 浏览器逐秒截图后合成 mp4","parameters":{"type":"object","properties":{"html":{"type":"string","description":"输入 html 路径"},"output":{"type":"string","description":"输出 mp4 路径"},"seconds":{"type":"integer","description":"视频秒数"}},"required":["html","output"]}}},{"type":"function","function":{"name":"rembg_remove","description":"抠图: 去除图片背景, 输出透明背景 png","parameters":{"type":"object","properties":{"image":{"type":"string","description":"输入图片路径"},"output":{"type":"string","description":"输出 png 路径"}},"required":["image","output"]}}},{"type":"function","function":{"name":"llm_generate","description":"通用文本创作/开放生成: 写文案、故事、诗、邮件、翻译、润色、起标题等纯文字创作, 不涉及具体软件操作, 不落文件","parameters":{"type":"object","properties":{"prompt":{"type":"string","description":"生成需求"}},"required":["prompt"]}}},{"type":"function","function":{"name":"video_info","description":"查看视频信息: 时长/分辨率/帧率","parameters":{"type":"object","properties":{"video":{"type":"string","description":"视频路径"}},"required":["video"]}}},{"type":"function","function":{"name":"shell_exec","description":"执行 shell 命令: 跑脚本/启动程序/查看系统状态 (跨平台 brush/nushell)","parameters":{"type":"object","properties":{"command":{"type":"string","description":"要执行的命令"},"cwd":{"type":"string","description":"工作目录(可选)"}},"required":["command"]}}},{"type":"function","function":{"name":"file_write","description":"把文本写入文件(自动建父目录): 生成启动脚本/配置文件","parameters":{"type":"object","properties":{"path":{"type":"string","description":"输出文件路径"},"content":{"type":"string","description":"文件内容"}},"required":["path","content"]}}},{"type":"function","function":{"name":"schedule","description":"创建定时任务(cron/计划任务): 例如每天 8:00 自动启动某程序","parameters":{"type":"object","properties":{"spec":{"type":"string","description":"计划表达式, 如 0 8 * * *"},"command":{"type":"string","description":"定时执行的命令"},"apply":{"type":"boolean","description":"是否立即登记(默认 false 仅返回配置)"}},"required":["spec","command"]}}}]"#;

/// 权威工具 schema (OpenAI tools 数组) — 供 `lycore tools` 导出, 单一真源防漂移
pub fn chat_tools() -> serde_json::Value {
    serde_json::from_str(CHAT_TOOLS).expect("CHAT_TOOLS 必须是合法 JSON")
}

pub struct LlamaCppBackend {
    client: reqwest::blocking::Client,
    base_url: String,
    model: String,
    /// 非 thinking 模式 (Qwen3: 工具调用场景必须关, 否则 token 耗在 <think>)
    pub enable_thinking: bool,
    /// 采样温度。FC 编排场景默认 0.1 —— 0.7 会让同一请求在多次运行间挑中不同工具,
    /// 表现为"同一个问题有时调 lyv_knowledge 有时不调", 且不可复现。
    /// 改写/润色等需要多样性的场景用 with_temperature() 调高。
    pub temperature: f32,
}

impl LlamaCppBackend {
    pub fn new(base_url: &str, model: &str) -> Self {
        crate::tools_runtime::install_crypto_provider();
        Self {
            client: reqwest::blocking::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            enable_thinking: false,
            temperature: 0.1,
        }
    }

    /// 覆盖采样温度 (改写/润色等需要多样性的场景; agent 编排不要调)
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
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
            "temperature": self.temperature,
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
