//! 技能描述符 (Skill descriptor) — `{ capabilities, risk, verifier, executor }`
//!
//! 设计来源: `docs/research-architecture-2026-09-16.md` P0。
//! 把 `verify.rs` (verifier) 与 `executor.rs` (executor) 提升为结构化 Skill,
//! 供模型-escalation / 权限裁决 / ToolRAG 裁剪统一使用。
//!
//! 关键解耦: `Skill` 不重复实现执行逻辑, 只做编排层的**结构化描述** —
//! `executor` 字段直接复用 `executor::Executor::execute(name, args)` 的工具名,
//! `verifier` 字段指向 `verify.rs` 的某个确定性级联 (P1 才接具体调用)。

use crate::capability::{self, Capability};

/// 风险等级 (技能越权 / 失败的爆炸半径)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

impl RiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            RiskLevel::Low => "Low",
            RiskLevel::Medium => "Medium",
            RiskLevel::High => "High",
        }
    }
}

/// 验证器标识 — 指向 `verify.rs` 的某个确定性级联
///
/// P1 把每个变体接成 `verify::*` 的真实调用 (见 research-architecture-2026-09-16.md
/// §1.3 Verifier 注册表)。`VerifierId::instantiate()` 返回 `Box<dyn Verifier>`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifierId {
    /// OCR 级联 (`verify::OcrVerifier` → `verify::verify`) — 识图类技能用
    Ocr,
    /// ffprobe 验证 (`verify::FfprobeVerifier`) — 视频产物确定性验收
    Ffprobe,
    /// VNN Rust 路径未实现 → 诚实降级到学习队列 (`verify::VnnVerifier`)
    Vnn,
    /// 退出码验证 (`verify::ExitCodeVerifier`) — shell 类技能用
    ExitCode,
    /// 文件存在且非空 (`verify::FileExistsVerifier`) — 落盘类技能用
    FileExists,
    /// 无确定性验证 (纯文本创作 / 检索) — 视为永远 pass
    None,
}

/// 技能 = 能力 + 风险 + 验证器 + 执行器(工具名)
///
/// 这是简报「Skill 描述符」的 Rust 落地。注册表 [`ALL_SKILLS`] 与
/// `llamacpp::CHAT_TOOLS` 七工具一一对应, 单一真源。
///
/// `desc` 是 P1 新增的**语义召回语料**: 含同义词/中英文, 供 ToolRAG 的
/// embedding 召回层 (toolrag.rs) 检索用, 与 `capabilities`/`risk` 解耦。
pub struct Skill {
    pub name: &'static str,
    pub capabilities: &'static [Capability],
    pub risk: RiskLevel,
    pub verifier: VerifierId,
    /// 执行器工具名 (与 `executor::Executor::execute` 的 match 分支一致)
    pub executor: &'static str,
    /// 语义描述 + 同义词 (ToolRAG 召回语料, 与 CHAT_TOOLS description 互补)
    pub desc: &'static str,
}

/// 全部技能注册表 (与 `llamacpp::CHAT_TOOLS` 七工具一一对应)
pub const ALL_SKILLS: &[Skill] = &[
    Skill {
        name: "lyv_knowledge",
        capabilities: &[Capability::FileRead],
        risk: RiskLevel::Low,
        verifier: VerifierId::None,
        executor: "lyv_knowledge",
        desc: "查询视频教程里演示过的具体操作命令步骤 软件怎么用 某命令怎么敲 教程怎么做 操作指南 howto tutorial 知识检索",
    },
    Skill {
        name: "vnn_identify",
        capabilities: &[Capability::FileRead, Capability::Camera],
        risk: RiskLevel::Medium,
        // VNN Rust 路径占位 (降级保持) — P1 接成 verify::VnnVerifier
        verifier: VerifierId::Vnn,
        executor: "vnn_identify",
        desc: "识别图片内容 截图分类 终端 GUI 自然 文档 画面理解 image classification scene recognition 识图",
    },
    Skill {
        name: "rembg_remove",
        capabilities: &[Capability::FileRead, Capability::FileWrite, Capability::Shell],
        risk: RiskLevel::Medium,
        verifier: VerifierId::None,
        executor: "rembg_remove",
        desc: "抠图 去除图片背景 透明背景 png 去背 matting cutout remove background 背景消除",
    },
    Skill {
        name: "html_gen",
        capabilities: &[Capability::FileWrite],
        risk: RiskLevel::Low,
        verifier: VerifierId::None,
        executor: "html_gen",
        desc: "生成 html 页面 单文件网页 前端页面 写网页 generate webpage 网页生成 静态页面",
    },
    Skill {
        name: "llm_generate",
        capabilities: &[Capability::Network],
        risk: RiskLevel::Low,
        verifier: VerifierId::None,
        executor: "llm_generate",
        desc: "文本创作 写文案 故事 诗 邮件 翻译 润色 起标题 开放生成 writing generation 写作 生成",
    },
    Skill {
        name: "html_render_video",
        capabilities: &[Capability::FileWrite, Capability::Shell],
        risk: RiskLevel::Medium,
        // P1 接成 verify::FfprobeVerifier — 视频产物确定性验收
        verifier: VerifierId::Ffprobe,
        executor: "html_render_video",
        desc: "把 html 页面渲染成视频 headless 浏览器截图合成 mp4 录屏网页 webpage to video render 网页转视频",
    },
    Skill {
        name: "video_info",
        capabilities: &[Capability::FileRead],
        risk: RiskLevel::Low,
        verifier: VerifierId::None,
        executor: "video_info",
        desc: "查看视频信息 时长 分辨率 帧率 视频元数据 video metadata duration 视频属性",
    },
    // --- 执行类 (日常任务)。Verifier 挂确定性验收: shell→退出码, 落盘→文件存在 ---
    Skill {
        name: "shell_exec",
        capabilities: &[Capability::Shell],
        risk: RiskLevel::High,
        verifier: VerifierId::ExitCode,
        executor: "shell_exec",
        desc: "执行 shell 命令 跑脚本 启动程序 让某程序运行起来 执行命令 run command shell script launch 跨平台 brush nushell",
    },
    Skill {
        name: "file_write",
        capabilities: &[Capability::FileWrite],
        risk: RiskLevel::Medium,
        verifier: VerifierId::FileExists,
        executor: "file_write",
        desc: "写文件 生成脚本 保存配置 输出到文件 落盘 write file save script config",
    },
    Skill {
        name: "schedule",
        capabilities: &[Capability::Shell, Capability::FileWrite],
        risk: RiskLevel::Medium,
        verifier: VerifierId::None,
        executor: "schedule",
        desc: "定时任务 定时启动 每天几点 定时执行 cron 计划任务 schedule timer daily 到点自动运行",
    },
];

/// 技能注册表查询 / 按能力过滤 (模型-escalation 与 ToolRAG 裁剪前置)
pub struct SkillRegistry;

impl SkillRegistry {
    /// 按名查技能
    pub fn lookup(name: &str) -> Option<&'static Skill> {
        ALL_SKILLS.iter().find(|s| s.name == name)
    }

    /// 按「可用能力集」过滤技能 — 模型-escalation 时只把授权工具交给该档位
    ///
    /// 这是 TinyAgent ToolRAG 范式的前置: 先按能力裁剪, 再按语义召回
    /// (research-architecture-2026-09-16.md §1.4)。
    pub fn filter_by_capabilities(available: &[Capability]) -> Vec<&'static Skill> {
        ALL_SKILLS
            .iter()
            .filter(|s| capability::grants(available, s.capabilities))
            .collect()
    }

    /// 导出过滤后技能的工具名 (喂给 `CHAT_TOOLS` 裁剪 / ToolRAG 召回源)
    pub fn allowed_tool_names(available: &[Capability]) -> Vec<&'static str> {
        Self::filter_by_capabilities(available)
            .iter()
            .map(|s| s.executor)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Capability;

    #[test]
    fn registry_matches_tool_caps() {
        // ALL_SKILLS 每个技能的能力必须与其 TOOL_CAPS 条目一致
        for s in ALL_SKILLS {
            assert_eq!(
                s.capabilities,
                crate::capability::capabilities_of(s.name),
                "技能 {} 的能力与 TOOL_CAPS 不一致",
                s.name
            );
        }
    }

    #[test]
    fn tiny_model_sees_only_fileread_tools() {
        let tiny = [Capability::FileRead];
        let allowed = SkillRegistry::allowed_tool_names(&tiny);
        // 只应有纯 FileRead 技能: lyv_knowledge, video_info
        assert!(allowed.contains(&"lyv_knowledge"));
        assert!(allowed.contains(&"video_info"));
        // 需要 Shell/Network/Camera 的必须被裁掉
        assert!(!allowed.contains(&"rembg_remove")); // Shell
        assert!(!allowed.contains(&"llm_generate")); // Network
        assert!(!allowed.contains(&"vnn_identify")); // Camera
    }

    #[test]
    fn full_model_sees_all() {
        let full = [
            Capability::FileRead,
            Capability::FileWrite,
            Capability::Shell,
            Capability::Network,
            Capability::Camera,
            Capability::DeviceControl,
            Capability::GitHub,
        ];
        assert_eq!(SkillRegistry::allowed_tool_names(&full).len(), 10);
    }
}
