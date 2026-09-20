//! v13 路由器接线 — MVP 能力①: 把自然语言需求变成**一条可执行的 CLI 命令**
//!
//! ## 链路
//! ```text
//! NL → v13 模型 → 命令文本 → 归一化(剥 think/brush 前缀) → T1 门 → 决策 → brush 执行
//! ```
//!
//! ## 三条硬纪律 (都是踩过的坑)
//! 1. **system prompt 必须与训练逐字一致** (含 brush 通用 shell 域), 且 `enable_thinking=false` —
//!    不一致会直接掉点; 开着思考模型会先长篇推理而不输出命令。
//! 2. **temperature = 0** — 路由器要的是确定性, 0.7 会让同一请求产出不同命令。
//! 3. **模型层不负责拒绝** — reject 集合实测 0%, 危险命令照样输出且最自信。
//!    安全判定全部在 `t1gate`, 这里只做"该不该放行"的分诊, 不做"命令对不对"的判断。
//!
//! 模型推理抽象为 `CommandModel` trait: 本机单测用 Scripted (不碰模型),
//! 真实部署用 `LlamaCppRouter` (llama-server HTTP, 通常在远端 GPU 上)。

use crate::t1gate::{classify, normalize, Risk, Verdict};

/// v13 训练时的 system prompt — 逐字对齐, 改一个字都要重训
pub const V13_SYS: &str = "你是 lyco_agent 的命令路由器。把用户的日常意图翻译成**一条**本地 CLI 命令。\
支持的域：hw(硬件)/gh(github)/ff(ffmpeg)/lb(行情持仓只读)/brush(shell 通用命令)。\
只输出命令本身，不要解释；不支持的请求输出 (无需调用硬件命令)。";

/// 模型说"这活儿不该用命令干"
pub const NOOP: &str = "(无需调用硬件命令)";

/// 模型 IO 抽象 (便于无模型单测 + 远端/端侧多种后端)
pub trait CommandModel {
    fn infer(&mut self, system: &str, user: &str) -> anyhow::Result<String>;
}

/// llama-server (OpenAI 兼容 HTTP) 后端
pub struct LlamaCppRouter {
    client: reqwest::blocking::Client,
    base_url: String,
    model: String,
}

impl LlamaCppRouter {
    pub fn new(base_url: &str, model: &str) -> Self {
        crate::tools_runtime::install_crypto_provider();
        Self {
            client: reqwest::blocking::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
        }
    }

    pub fn health(&self) -> anyhow::Result<()> {
        let r = self
            .client
            .get(format!("{}/health", self.base_url))
            .send()?
            .status();
        anyhow::ensure!(r.is_success(), "llama-server unhealthy: {r}");
        Ok(())
    }
}

impl CommandModel for LlamaCppRouter {
    fn infer(&mut self, system: &str, user: &str) -> anyhow::Result<String> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            // 路由器: 零温度(确定性) + 短上限(只要一条命令) + 关思考
            "temperature": 0.0,
            "max_tokens": 64,
            "chat_template_kwargs": {"enable_thinking": false},
        });
        let v: serde_json::Value = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&body)
            .send()?
            .error_for_status()?
            .json()?;
        Ok(v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string())
    }
}

/// 路由结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct Route {
    /// 归一化后的命令 (剥掉 think 块与 brush 前缀)
    pub command: String,
    /// 模型原始输出 (排障用)
    pub raw: String,
    /// 模型判定"这不需要命令"
    pub is_noop: bool,
}

/// 分诊决策
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Decision {
    /// 只读 — 直接执行
    Run,
    /// 写操作 — 需 T1 确认后才执行
    Confirm,
    /// 破坏性 — 拦截 (需显式授权才可能放行)
    Block,
    /// 与命令无关的请求 — 不执行
    Noop,
}

/// 完整执行计划: NL → 命令 → 风险 → 决策
#[derive(Debug, Clone, serde::Serialize)]
pub struct Plan {
    pub nl: String,
    pub command: String,
    pub raw: String,
    pub risk: Risk,
    pub decision: Decision,
    pub verdict: Verdict,
}

/// 主入口: 问一句 → 出一份执行计划 (不执行)
pub fn plan(model: &mut dyn CommandModel, nl: &str) -> anyhow::Result<Plan> {
    let raw = model.infer(V13_SYS, nl)?;
    Ok(plan_from_raw(nl, &raw))
}

/// 从已知模型输出构造计划 (远端批量跑模型 → 本地分诊时用这个)
pub fn plan_from_raw(nl: &str, raw: &str) -> Plan {
    let command = normalize(raw);
    let is_noop = command.contains("无需调用");
    let verdict = classify(&command);
    let decision = if is_noop {
        Decision::Noop
    } else {
        match verdict.risk {
            Risk::Read => Decision::Run,
            Risk::Write => Decision::Confirm,
            Risk::Danger => Decision::Block,
        }
    };
    Plan {
        nl: nl.to_string(),
        command,
        raw: raw.to_string(),
        risk: verdict.risk,
        decision,
        verdict,
    }
}

/// 是否已获授权 (写操作需 confirmed, 危险命令需 confirmed + override)
pub fn may_execute(p: &Plan, confirmed: bool, danger_override: bool) -> bool {
    match p.decision {
        Decision::Run => true,
        Decision::Confirm => confirmed,
        Decision::Block => confirmed && danger_override,
        Decision::Noop => false,
    }
}

/// 执行 (已分诊通过才调) — 走 brush; 调用方负责先过 may_execute
pub fn execute(p: &Plan, cwd: Option<&std::path::Path>) -> crate::tools_runtime::RunOutcome {
    crate::tools_runtime::shell_exec(&p.command, cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回放后端: 不加载模型, 直接返回预设输出 (本机单测绝不跑真模型)
    struct Scripted {
        out: String,
        seen_system: String,
    }
    impl CommandModel for Scripted {
        fn infer(&mut self, system: &str, _user: &str) -> anyhow::Result<String> {
            self.seen_system = system.to_string();
            Ok(self.out.clone())
        }
    }

    fn scripted(out: &str) -> Scripted {
        Scripted {
            out: out.to_string(),
            seen_system: String::new(),
        }
    }

    #[test]
    fn system_prompt_matches_training() {
        let mut m = scripted("gh issue list");
        let _ = plan(&mut m, "列出没关的 issue").unwrap();
        assert_eq!(m.seen_system, V13_SYS);
        assert!(V13_SYS.contains("brush(shell 通用命令)"), "brush 域必须在 prompt 里");
    }

    #[test]
    fn normalizes_brush_prefixed_output() {
        // 模型实测输出形态: `brush <cmd>`
        let p = plan_from_raw("列出没关的 issue", "brush gh issue list");
        assert_eq!(p.command, "gh issue list");
        assert_eq!(p.decision, Decision::Run);
    }

    #[test]
    fn strips_think_block_before_classify() {
        let p = plan_from_raw("x", "<think>想想\n</think>\ngh pr list");
        assert_eq!(p.command, "gh pr list");
        assert_eq!(p.decision, Decision::Run);
    }

    #[test]
    fn read_requests_run_directly() {
        for (nl, raw) in [
            ("看看有哪些 issue", "brush gh issue list"),
            ("当前状态", "brush git status"),
            ("编译检查一下", "brush cargo check"),
        ] {
            let p = plan_from_raw(nl, raw);
            assert_eq!(p.decision, Decision::Run, "{nl} / {raw}");
            assert!(may_execute(&p, false, false));
        }
    }

    #[test]
    fn write_requests_need_confirm() {
        let p = plan_from_raw("装个 serde", "brush cargo add serde");
        assert_eq!(p.decision, Decision::Confirm);
        assert!(!may_execute(&p, false, false), "未确认不得执行");
        assert!(may_execute(&p, true, false), "确认后可执行");
    }

    /// 核心: 模型照常吐出的危险命令, 必须被分诊为 Block (reject 0% 的唯一防线)
    #[test]
    fn danger_requests_blocked_even_with_confirm() {
        for raw in [
            "brush cargo publish",
            "brush cargo clean",
            "rm -rf target",
            "git push --force",
        ] {
            let p = plan_from_raw("发布/清理", raw);
            assert_eq!(p.decision, Decision::Block, "{raw}");
            assert!(!may_execute(&p, true, false), "仅确认不足以放行危险命令: {raw}");
            assert!(may_execute(&p, true, true), "确认+显式 override 才行: {raw}");
        }
    }

    #[test]
    fn noop_is_not_executed() {
        let p = plan_from_raw("今天天气怎么样", NOOP);
        assert_eq!(p.decision, Decision::Noop);
        assert!(!may_execute(&p, true, true), "noop 永远不执行");
    }
}
