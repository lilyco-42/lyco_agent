//! 能力层 (Capability Layer) — agent 的一等公民组件
//!
//! 设计来源: `docs/research-architecture-2026-09-16.md` P0。
//! 简报主张: agent 不应直接暴露「能执行什么命令」, 而应声明一组抽象能力
//! (FileRead/Shell/Network/Camera/DeviceControl/GitHub …), 由运行时把能力映射到
//! 具体权限与工具。模型/调度层只看得见能力, 看不见危险原语。
//!
//! 工具 → 能力集在 [`TOOL_CAPS`] 单一声明, 与 `llamacpp::CHAT_TOOLS` 同生命周期维护,
//! 避免「schema 有了、能力没登记」的漂移。

/// 抽象能力 (不是具体命令 / 工具)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    FileRead,
    FileWrite,
    Shell,
    Network,
    Camera,
    DeviceControl,
    GitHub,
}

impl Capability {
    /// 人类可读名 (日志 / 调试 / 序列化)
    pub fn as_str(&self) -> &'static str {
        match self {
            Capability::FileRead => "FileRead",
            Capability::FileWrite => "FileWrite",
            Capability::Shell => "Shell",
            Capability::Network => "Network",
            Capability::Camera => "Camera",
            Capability::DeviceControl => "DeviceControl",
            Capability::GitHub => "GitHub",
        }
    }
}

/// 工具 → 能力集 (与 `llamacpp::CHAT_TOOLS` 七工具严格对应)
///
/// 这是能力声明的唯一真源。新增工具必须在此登记, 否则
/// [`filter_by_capabilities`](crate::skill::SkillRegistry::filter_by_capabilities)
/// 永远看不到它 (默认零能力 = 任何档位都不授权)。
pub const TOOL_CAPS: &[(&str, &[Capability])] = &[
    ("lyv_knowledge", &[Capability::FileRead]),
    ("vnn_identify", &[Capability::FileRead, Capability::Camera]),
    (
        "rembg_remove",
        &[Capability::FileRead, Capability::FileWrite, Capability::Shell],
    ),
    ("html_gen", &[Capability::FileWrite]),
    ("llm_generate", &[Capability::Network]),
    (
        "html_render_video",
        &[Capability::FileWrite, Capability::Shell],
    ),
    ("video_info", &[Capability::FileRead]),
    // --- 执行类工具 (日常任务"真能干活"所需; 用户 2026-09-16 要求) ---
    ("shell_exec", &[Capability::Shell]),
    ("file_write", &[Capability::FileWrite]),
    ("schedule", &[Capability::Shell, Capability::FileWrite]),
];

/// 查询某工具声明的能力 (未登记返回空切片)
pub fn capabilities_of(tool: &str) -> &'static [Capability] {
    for (name, caps) in TOOL_CAPS {
        if *name == tool {
            return caps;
        }
    }
    &[]
}

/// 能力子集判定: `granted` 是否涵盖 `required` 全部
///
/// 模型-escalation 时: 某档位被授予一组能力, 只有当它覆盖技能所需的全部能力,
/// 该技能才对该档位可见。
pub fn grants(granted: &[Capability], required: &[Capability]) -> bool {
    required.iter().all(|r| granted.contains(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_caps_match_chat_tools() {
        // 七工具必须全在 TOOL_CAPS 中 (与 CHAT_TOOLS 一致)
        let names = [
            "lyv_knowledge",
            "vnn_identify",
            "rembg_remove",
            "html_gen",
            "llm_generate",
            "html_render_video",
            "video_info",
            "shell_exec",
            "file_write",
            "schedule",
        ];
        for n in names {
            assert!(
                !capabilities_of(n).is_empty(),
                "工具 {n} 未在 TOOL_CAPS 登记能力"
            );
        }
    }

    #[test]
    fn grants_subset_semantics() {
        let full = [
            Capability::FileRead,
            Capability::FileWrite,
            Capability::Shell,
            Capability::Network,
            Capability::Camera,
        ];
        // 小模型只有 FileRead → 看不到需要 Shell 的 rembg_remove
        let tiny = [Capability::FileRead];
        assert!(grants(&full, &[Capability::Shell, Capability::FileWrite]));
        assert!(!grants(&tiny, &[Capability::Shell]));
        // 空需求任何授权都满足
        assert!(grants(&tiny, &[]));
    }
}
