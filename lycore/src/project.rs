//! project — 项目目录扫描 + 启动脚本生成 (端到端「从目录到能跑」)
//!
//! 支撑用户指令: *"写一个 8:00-22:00 自动启动 java 起 Minecraft 的程序"*。
//! 这条指令的能力链:
//!
//!   1. 学 java CLI 参数        → `learn_cli::index_and_build("java", pack)` (已有)
//!   2. **扫项目目录认得 paper.jar** → 本模块 [`scan`]
//!   3. **关联 Minecraft/paper 知识** → 本模块 [`knowledge_cues`] + [`MINECRAFT_KNOWLEDGE`]
//!   4. **生成带时间窗的启动脚本**   → 本模块 [`launch_script`]
//!   5. NPU + 视频 + ASR 学习     → `learn` 管线 + `npu_runtime` (已有)
//!
//! 设计: 纯文件系统扫描, 零外部依赖; 识别依据是 jar 文件名约定 (paper/spigot/
//! fabric/forge/server) —— 与真实 Minecraft 生态一致, 无需读 jar 内容。

use crate::learn::Cue;
use std::path::Path;

/// 服务端类型 (按 jar 名约定识别)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerKind {
    Paper,
    Spigot,
    Fabric,
    Forge,
    Vanilla,
    Unknown,
}

impl ServerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServerKind::Paper => "Paper",
            ServerKind::Spigot => "Spigot",
            ServerKind::Fabric => "Fabric",
            ServerKind::Forge => "Forge",
            ServerKind::Vanilla => "Vanilla",
            ServerKind::Unknown => "Unknown",
        }
    }

    /// 优先级 (同名多 jar 时取最高), Paper 生态最常用
    fn rank(&self) -> u8 {
        match self {
            ServerKind::Paper => 5,
            ServerKind::Spigot => 4,
            ServerKind::Fabric => 3,
            ServerKind::Forge => 2,
            ServerKind::Vanilla => 1,
            ServerKind::Unknown => 0,
        }
    }

    fn from_jar(name: &str) -> ServerKind {
        let n = name.to_lowercase();
        if n.contains("paper") {
            ServerKind::Paper
        } else if n.contains("spigot") {
            ServerKind::Spigot
        } else if n.contains("fabric") {
            ServerKind::Fabric
        } else if n.contains("forge") {
            ServerKind::Forge
        } else if n.contains("server") || n.contains("minecraft") {
            ServerKind::Vanilla
        } else {
            ServerKind::Unknown
        }
    }
}

/// 目录中的一件产物
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub name: String,
    /// "server-jar" | "jar" | "plugins-dir" | "world" | "config"
    pub kind: &'static str,
}

/// 项目扫描结果
#[derive(Debug, Clone)]
pub struct ProjectScan {
    pub dir: String,
    pub kind: ServerKind,
    pub server_jar: Option<String>,
    pub artifacts: Vec<Artifact>,
    pub eula_present: bool,
    pub plugins_dir: bool,
    pub ram_mb: u32,
}

/// 扫描目录: 识别服务端 jar / 插件目录 / 世界 / 配置
pub fn scan(dir: &Path) -> std::io::Result<ProjectScan> {
    let mut artifacts = Vec::new();
    let mut best: Option<(ServerKind, String)> = None;
    let mut eula_present = false;
    let mut plugins_dir = false;

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        if path.is_dir() {
            if name.eq_ignore_ascii_case("plugins") {
                plugins_dir = true;
                artifacts.push(Artifact { name, kind: "plugins-dir" });
            } else if matches!(name.as_str(), "world" | "world_nether" | "world_the_end") {
                artifacts.push(Artifact { name, kind: "world" });
            }
            continue;
        }
        let lower = name.to_lowercase();
        if lower.ends_with(".jar") {
            let k = ServerKind::from_jar(&name);
            if k != ServerKind::Unknown {
                let better = best.as_ref().map(|(bk, _)| k.rank() > bk.rank()).unwrap_or(true);
                if better {
                    best = Some((k, name.clone()));
                }
                artifacts.push(Artifact { name, kind: "server-jar" });
            } else {
                artifacts.push(Artifact { name, kind: "jar" });
            }
        } else if lower == "eula.txt" {
            eula_present = true;
            artifacts.push(Artifact { name, kind: "config" });
        } else if lower == "server.properties" {
            artifacts.push(Artifact { name, kind: "config" });
        }
    }

    let (kind, server_jar) = match best {
        Some((k, n)) => (k, Some(n)),
        None => (ServerKind::Unknown, None),
    };
    Ok(ProjectScan {
        dir: dir.display().to_string(),
        kind,
        server_jar,
        artifacts,
        eula_present,
        plugins_dir,
        ram_mb: 2048,
    })
}

/// 内置 Minecraft/Paper 领域知识 (关联知识, 随项目一起入知识包)
pub const MINECRAFT_KNOWLEDGE: &[(&str, &str)] = &[
    (
        "paper minecraft",
        "Minecraft 服务端 (PaperMC, 高性能 Spigot 分支): 启动 java -Xmx2G -jar paper.jar nogui; \
         首次须在 eula.txt 写 eula=true 同意 EULA; 插件放 plugins/; 默认端口 25565",
    ),
    (
        "minecraft server 启动",
        "启动命令 java -Xms2G -Xmx2G -jar <server.jar> nogui; nogui 关图形界面; \
         内存按在线人数调; Java 版本须匹配 (1.20.5+ 用 Java 21, 1.17-1.20 用 Java 17)",
    ),
    (
        "minecraft 定时启停",
        "定时启动: Linux cron `0 8 * * * /path/start.sh`; Windows `schtasks /Create /SC DAILY /ST 08:00`; \
         关服 Linux `pkill -f paper.jar` / Windows `taskkill /F /IM java.exe`",
    ),
];

/// 由扫描结果生成知识 Cue (通用 paper 知识 + 本项目特有事实)
pub fn knowledge_cues(scan: &ProjectScan) -> Vec<Cue> {
    let mut cues = Vec::new();
    for (k, v) in MINECRAFT_KNOWLEDGE {
        cues.push(Cue {
            t0: 0.0,
            t1: 0.0,
            text: format!("{k} : {v}"),
        });
    }
    if let Some(jar) = &scan.server_jar {
        cues.push(Cue {
            t0: 0.0,
            t1: 0.0,
            text: format!(
                "项目 {} : {} 服务端 : 启动 java -Xmx{}M -jar {} nogui",
                scan.dir,
                scan.kind.as_str(),
                scan.ram_mb,
                jar
            ),
        });
    } else {
        cues.push(Cue {
            t0: 0.0,
            t1: 0.0,
            text: format!("项目 {} : 未发现服务端 jar : 需下载 paper.jar 后放入目录", scan.dir),
        });
    }
    if !scan.eula_present {
        cues.push(Cue {
            t0: 0.0,
            t1: 0.0,
            text: format!(
                "项目 {} : 缺 eula.txt : 首次启动须生成 eula.txt 内容 eula=true 才可运行",
                scan.dir
            ),
        });
    }
    cues
}

/// "8:00" / "0800" / "22:00" → 紧凑数字 (0800) 再转十进制整数 (800)
fn hhmm_num(t: &str) -> u32 {
    let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    match digits.len() {
        0 => 0,
        1 | 2 => digits.parse::<u32>().unwrap_or(0) * 100,
        _ => {
            let split = digits.len() - 2;
            let h = digits[..split].parse::<u32>().unwrap_or(0);
            let m = digits[split..].parse::<u32>().unwrap_or(0);
            h * 100 + m
        }
    }
}

/// 生成跨平台启停脚本 (含时间窗守卫 + cron/schtasks 片段)
pub fn launch_script(scan: &ProjectScan, start: &str, end: &str) -> String {
    let jar = scan.server_jar.clone().unwrap_or_else(|| "paper.jar".to_string());
    let ram = scan.ram_mb;
    let sh = hhmm_num(start);
    let eh = hhmm_num(end);
    format!(
        r#"#!/usr/bin/env bash
# lyco 自动启动脚本 — {kind} 服务端 ({jar})
# 时间窗: {start} - {end}  (窗外自动退出)
set -u
H=$(date +%H%M); H=$((10#$H))
if [ "$H" -lt {sh} ] || [ "$H" -gt {eh} ]; then
  echo "[lyco] 非运行时段 ({start}-{end}), 跳过"; exit 0
fi
cd "$(dirname "$0")" || exit 1
[ -f eula.txt ] || echo "eula=true" > eula.txt   # 首次自动同意 EULA
exec java -Xms{ram}M -Xmx{ram}M -jar "{jar}" nogui

# ================= 定时调度 (二选一) =================
# --- Linux (crontab -e) ---
# 0 8  * * * cd {dir} && ./start_mc.sh >> mc.log 2>&1   # 08:00 启动
# 0 22 * * * pkill -f '{jar}'                          # 22:00 关闭
# --- Windows (管理员 CMD) ---
# schtasks /Create /SC DAILY /ST {start} /TN MCStart /TR "java -Xms{ram}M -Xmx{ram}M -jar {jar} nogui"
# schtasks /Create /SC DAILY /ST {end} /TN MCStop  /TR "taskkill /F /IM java.exe"
"#,
        kind = scan.kind.as_str(),
        jar = jar,
        ram = ram,
        start = start,
        end = end,
        sh = sh,
        eh = eh,
        dir = scan.dir,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mk(dir: &Path, files: &[&str], dirs: &[&str]) -> PathBuf {
        for d in dirs {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        for f in files {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        dir.to_path_buf()
    }

    #[test]
    fn scan_detects_paper_server() {
        let t = tempfile::tempdir().unwrap();
        mk(t.path(), &["paper.jar", "eula.txt", "server.properties"], &["plugins", "world"]);
        let s = scan(t.path()).unwrap();
        assert_eq!(s.kind, ServerKind::Paper);
        assert_eq!(s.server_jar.as_deref(), Some("paper.jar"));
        assert!(s.eula_present, "eula.txt 应被识别");
        assert!(s.plugins_dir, "plugins/ 应被识别");
        assert_eq!(s.ram_mb, 2048);
        assert!(s.artifacts.iter().any(|a| a.name == "world" && a.kind == "world"));
    }

    #[test]
    fn scan_prefers_paper_over_vanilla() {
        let t = tempfile::tempdir().unwrap();
        mk(t.path(), &["minecraft_server.jar", "paper.jar"], &[]);
        let s = scan(t.path()).unwrap();
        assert_eq!(s.kind, ServerKind::Paper, "同名多 jar 取优先级最高");
        assert_eq!(s.server_jar.as_deref(), Some("paper.jar"));
    }

    #[test]
    fn scan_detects_fabric() {
        let t = tempfile::tempdir().unwrap();
        mk(t.path(), &["fabric-server-launch.jar"], &["mods"]);
        let s = scan(t.path()).unwrap();
        assert_eq!(s.kind, ServerKind::Fabric);
        assert_eq!(s.server_jar.as_deref(), Some("fabric-server-launch.jar"));
    }

    #[test]
    fn scan_empty_dir_is_unknown() {
        let t = tempfile::tempdir().unwrap();
        let s = scan(t.path()).unwrap();
        assert_eq!(s.kind, ServerKind::Unknown);
        assert!(s.server_jar.is_none());
        assert!(!s.eula_present);
    }

    #[test]
    fn hhmm_num_normalizes() {
        assert_eq!(hhmm_num("8:00"), 800);
        assert_eq!(hhmm_num("08:00"), 800);
        assert_eq!(hhmm_num("2200"), 2200);
        assert_eq!(hhmm_num("22:00"), 2200);
    }

    #[test]
    fn launch_script_has_window_jar_and_schedulers() {
        let t = tempfile::tempdir().unwrap();
        mk(t.path(), &["paper.jar", "eula.txt"], &["plugins"]);
        let s = scan(t.path()).unwrap();
        let sh = launch_script(&s, "08:00", "22:00");
        assert!(sh.contains("paper.jar"), "脚本须含 jar 名");
        assert!(sh.contains("08:00") && sh.contains("22:00"), "须含时间窗");
        assert!(sh.contains("-Xmx2048M"), "须含内存参数");
        assert!(sh.contains("nogui"), "须含 nogui");
        assert!(sh.contains("java"), "须是 java 启动");
        assert!(sh.contains("800") && sh.contains("2200"), "守卫用十进制 HHMM");
        assert!(sh.contains("10#$H"), "HHMM 须按十进制解析, 防 octal 坑");
        assert!(sh.contains("crontab") || sh.contains("schtasks"), "须含调度片段");
        assert!(sh.contains("eula=true"), "首次自动同意 EULA");
    }

    #[test]
    fn knowledge_cues_cover_paper_and_project_facts() {
        let t = tempfile::tempdir().unwrap();
        mk(t.path(), &["paper.jar"], &[]); // 无 eula.txt
        let s = scan(t.path()).unwrap();
        let cues = knowledge_cues(&s);
        let joined: String = cues.iter().map(|c| c.text.clone()).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("paper minecraft"), "须含 paper 领域知识");
        assert!(joined.contains("paper.jar"), "须含项目 jar 事实");
        assert!(joined.contains("eula.txt"), "缺 eula 须提示");
    }
}
