//! cli_zh —— CLI 说明书的**本地中文翻译**（零网络、零模型、确定性）。
//!
//! ## 它要解决什么
//!
//! `help_parse` 能把 `cli --help` 解析成动作表，但 `desc` 是**英文原文**：
//!
//! ```text
//! docker ps          List containers
//! docker logs        Fetch the logs of a container
//! docker build       Build an image from a Dockerfile
//! ```
//!
//! 目标是三类产物，**全部离线可复现**：
//! 1. **中文说明书** —— 给人看的（`lycore manual --cli docker`）；
//! 2. **初步训练数据** —— `(中文人话 → 可执行命令)` 的冷启动语料；
//! 3. **以后发展的底料** —— 将来模型要理解"中文 CLI 语义"，需要的就是这层对照。
//!
//! ## 为什么不用大模型翻译
//!
//! 与全项目的结论一致（`Δ_SFT = −43.3pp`：能力不在权重里）：
//!
//! * 端侧**必须离线**，云端翻译违背普惠不变量；
//! * 0.6B 做**信息抽取**不可靠（v17b 实测：让模型提炼 help → docker 掉前缀、cargo 吐 `cargo:build`）；
//! * 翻译结果会**成为训练数据**——数据里掺进模型的幻觉，等于把噪声训进权重（这正是 v13 失败的一个来源）。
//!
//! ## 诚实纪律（照抄 `help_parse` 的血泪教训）
//!
//! > 「解析出垃圾」比「解析出 0 条」更危险。
//!
//! 同理：**「翻译错」比「不翻译」更危险**。所以本模块：
//! * 只翻**词典命中的词**，未命中一律**原样保留英文**（绝不猜）；
//! * 每条都带 `coverage`（命中率）与 `unknown`（没翻的词）——**看懂多少是可量化的**；
//! * 译文里中文与英文原词混排，读者一眼能看出哪部分机器翻过。

use crate::help_parse::HelpAction;

/// 翻译结果 —— 永远保留英文原文，中文只是**附加信息**。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ZhText {
    pub en: String,
    pub zh: String,
    /// 命中词数 / 内容词总数（0.0–1.0）。未命中词原样保留，所以 **zh 非空不代表全译**。
    pub coverage: f64,
    /// 没查到的词（原样保留在 zh 里）
    pub unknown: Vec<String>,
    /// 是否走了句式模板（`X of Y` / `X from Y` 等）——比逐词拼更可信
    pub patterned: bool,
}

impl ZhText {
    fn fallback(en: &str) -> Self {
        Self {
            en: en.to_string(),
            zh: en.to_string(),
            coverage: 0.0,
            unknown: Vec::new(),
            patterned: false,
        }
    }
}

/// 动词表：help 描述的首词 → 中文动作。
///
/// 覆盖度是刻意的：按 CLI help 的真实用词分布挑的（list/show/create/remove/…），
/// 而不是按通用英语词频。查不到就退回英文，**不加新词猜**。
const VERBS: &[(&str, &str)] = &[
    ("list", "列出"),
    ("ls", "列出"),
    ("show", "显示"),
    ("display", "显示"),
    ("print", "打印"),
    ("get", "获取"),
    ("fetch", "获取"),
    ("read", "读取"),
    ("create", "创建"),
    ("new", "新建"),
    ("add", "添加"),
    ("insert", "插入"),
    ("remove", "删除"),
    ("delete", "删除"),
    ("rm", "删除"),
    ("destroy", "销毁"),
    ("update", "更新"),
    ("modify", "修改"),
    ("set", "设置"),
    ("edit", "编辑"),
    ("rename", "重命名"),
    ("move", "移动"),
    ("copy", "复制"),
    ("cp", "复制"),
    ("link", "关联"),
    ("unlink", "解除关联"),
    ("start", "启动"),
    ("stop", "停止"),
    ("restart", "重启"),
    ("run", "运行"),
    ("exec", "执行"),
    ("execute", "执行"),
    ("apply", "应用"),
    ("build", "构建"),
    ("compile", "编译"),
    ("install", "安装"),
    ("uninstall", "卸载"),
    ("upgrade", "升级"),
    ("pull", "拉取"),
    ("push", "推送"),
    ("clone", "克隆"),
    ("commit", "提交"),
    ("merge", "合并"),
    ("rebase", "变基"),
    ("revert", "回退"),
    ("reset", "重置"),
    ("checkout", "切换"),
    ("switch", "切换"),
    ("stash", "暂存"),
    ("tag", "打标签"),
    ("branch", "分支操作"),
    ("status", "查看状态"),
    ("info", "查看信息"),
    ("inspect", "查看详情"),
    ("describe", "查看详情"),
    ("logs", "查看日志"),
    ("log", "查看日志"),
    ("plan", "预览变更"),
    ("diff", "显示差异"),
    ("check", "检查"),
    ("test", "测试"),
    ("validate", "校验"),
    ("verify", "验证"),
    ("search", "搜索"),
    ("find", "查找"),
    ("query", "查询"),
    ("open", "打开"),
    ("close", "关闭"),
    ("export", "导出"),
    ("import", "导入"),
    ("generate", "生成"),
    ("init", "初始化"),
    ("clean", "清理"),
    ("prune", "清理废弃项"),
    ("scale", "扩缩容"),
    ("attach", "接入"),
    ("detach", "断开"),
    ("pause", "暂停"),
    ("unpause", "继续"),
    ("resume", "恢复"),
    ("kill", "终止"),
    ("wait", "等待"),
    ("watch", "持续监视"),
    ("top", "查看资源占用"),
    ("stats", "查看统计"),
    ("history", "查看历史"),
    ("version", "查看版本"),
    ("help", "查看帮助"),
    ("enable", "启用"),
    ("disable", "禁用"),
    ("mount", "挂载"),
    ("unmount", "卸载挂载"),
    ("launch", "启动"),
    ("restore", "恢复"),
    ("save", "保存"),
    ("load", "加载"),
    ("convert", "转换"),
    ("format", "格式化"),
    ("analyze", "分析"),
    ("trace", "跟踪"),
    ("complete", "补全"),
    ("release", "发布"),
    ("publish", "发布"),
    ("upload", "上传"),
    ("download", "下载"),
    ("sync", "同步"),
    ("dump", "导出"),
];

/// 名词表：CLI 领域里反复出现的实体。**专名（docker/kubectl/nginx）不在表内 → 原样保留**，
/// 这是刻意的：专名不该被翻译。
const NOUNS: &[(&str, &str)] = &[
    ("container", "容器"),
    ("containers", "容器"),
    ("image", "镜像"),
    ("images", "镜像"),
    ("volume", "数据卷"),
    ("volumes", "数据卷"),
    ("network", "网络"),
    ("networks", "网络"),
    ("service", "服务"),
    ("services", "服务"),
    ("node", "节点"),
    ("nodes", "节点"),
    ("pod", "Pod"),
    ("pods", "Pod"),
    ("deployment", "部署"),
    ("deployments", "部署"),
    ("namespace", "命名空间"),
    ("namespaces", "命名空间"),
    ("cluster", "集群"),
    ("clusters", "集群"),
    ("file", "文件"),
    ("files", "文件"),
    ("directory", "目录"),
    ("directories", "目录"),
    ("folder", "文件夹"),
    ("path", "路径"),
    ("paths", "路径"),
    ("branch", "分支"),
    ("branches", "分支"),
    ("commit", "提交"),
    ("commits", "提交"),
    ("tag", "标签"),
    ("tags", "标签"),
    ("label", "标签"),
    ("labels", "标签"),
    ("repository", "仓库"),
    ("repositories", "仓库"),
    ("repo", "仓库"),
    ("package", "软件包"),
    ("packages", "软件包"),
    ("module", "模块"),
    ("modules", "模块"),
    ("dependency", "依赖"),
    ("dependencies", "依赖"),
    ("process", "进程"),
    ("processes", "进程"),
    ("port", "端口"),
    ("ports", "端口"),
    ("log", "日志"),
    ("logs", "日志"),
    ("config", "配置"),
    ("configuration", "配置"),
    ("environment", "环境"),
    ("variable", "变量"),
    ("variables", "变量"),
    ("user", "用户"),
    ("users", "用户"),
    ("group", "用户组"),
    ("groups", "用户组"),
    ("permission", "权限"),
    ("permissions", "权限"),
    ("disk", "磁盘"),
    ("memory", "内存"),
    ("usage", "用量"),
    ("resource", "资源"),
    ("resources", "资源"),
    ("task", "任务"),
    ("tasks", "任务"),
    ("project", "项目"),
    ("projects", "项目"),
    ("token", "令牌"),
    ("secret", "密钥"),
    ("secrets", "密钥"),
    ("error", "错误"),
    ("errors", "错误"),
    ("output", "输出"),
    ("input", "输入"),
    ("history", "历史"),
    ("change", "变更"),
    ("changes", "变更"),
    ("version", "版本"),
    ("information", "信息"),
    ("details", "详情"),
    ("status", "状态"),
    ("state", "状态"),
    ("size", "大小"),
    ("template", "模板"),
    ("filter", "过滤条件"),
    ("format", "格式"),
    ("command", "命令"),
    ("commands", "命令"),
    ("argument", "参数"),
    ("arguments", "参数"),
    ("option", "选项"),
    ("options", "选项"),
    ("flag", "开关"),
    ("flags", "开关"),
    ("name", "名称"),
    ("names", "名称"),
    ("id", "ID"),
    ("ids", "ID"),
    ("address", "地址"),
    ("url", "地址"),
    ("host", "主机"),
    ("server", "服务器"),
    ("client", "客户端"),
    ("daemon", "守护进程"),
    ("registry", "镜像仓库"),
    ("driver", "驱动"),
    ("plugin", "插件"),
    ("plugins", "插件"),
    ("script", "脚本"),
    ("content", "内容"),
    ("message", "说明信息"),
    ("summary", "摘要"),
    ("report", "报告"),
    ("list", "列表"),
    ("entry", "条目"),
    ("entries", "条目"),
    ("item", "条目"),
    ("items", "条目"),
    ("value", "值"),
    ("values", "值"),
    ("key", "键"),
    ("keys", "键"),
    ("field", "字段"),
    ("fields", "字段"),
    ("rule", "规则"),
    ("rules", "规则"),
    ("policy", "策略"),
    ("timeout", "超时"),
    ("limit", "上限"),
    ("time", "时间"),
    ("date", "日期"),
    ("source", "源"),
    ("target", "目标"),
    ("destination", "目标位置"),
    ("default", "默认"),
    ("current", "当前"),
    ("local", "本地"),
    ("remote", "远程"),
    ("running", "运行中的"),
    ("stopped", "已停止的"),
    ("paused", "已暂停的"),
    ("all", "所有"),
    ("each", "每个"),
    ("one", "一个"),
    ("more", "若干"),
    ("number", "数量"),
    ("count", "计数"),
    ("binary", "二进制文件"),
    ("executable", "可执行文件"),
    ("archive", "归档"),
    ("layer", "层"),
    ("layers", "层"),
    ("context", "上下文"),
    ("dockerfile", "Dockerfile"),
    ("volume", "数据卷"),
];

/// 应被丢弃的功能词（不参与覆盖率统计，也不出现在译文里）
const DROP: &[&str] = &[
    "a", "an", "the", "this", "that", "these", "those", "is", "are", "be", "been", "to", "will",
    "it", "its", "as", "at", "by", "with", "in", "on", "up", "down", "out", "off", "and", "or",
    "but", "not", "only", "also", "from", "into", "for", "of", "do", "does", "done", "please",
];

/// 结构词（决定中文语序）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pivot {
    /// `X of Y` → `Y的X`
    Of,
    /// `X from Y` → `从Y X`
    From,
    /// `X to/into Y` → `把X 到Y`
    To,
}

fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

fn lookup(table: &[(&str, &str)], key: &str) -> Option<String> {
    table
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| (*v).to_string())
}

/// 把一串 token 逐词转中文。返回 (译文片段, 命中内容词数, 内容词总数, 未命中词)。
fn gloss(tokens: &[String]) -> (Vec<String>, usize, usize, Vec<String>) {
    let mut pieces: Vec<String> = Vec::new();
    let mut hit = 0usize;
    let mut total = 0usize;
    let mut unknown: Vec<String> = Vec::new();

    for t in tokens {
        let l = lower(t);
        if DROP.contains(&l.as_str()) && l.len() <= 4 {
            // 极短功能词直接丢（`a`/`an`/`the`）；长功能词（`please`）也丢
            continue;
        }
        if let Some(zh) = lookup(NOUNS, &l) {
            pieces.push(zh);
            hit += 1;
            total += 1;
            continue;
        }
        // 复数/时态兜底：生成词干候选，**由词典裁决**（谁在词典里谁赢）
        let stems = stem_candidates(&l);
        if let Some(zh) = stems
            .iter()
            .find_map(|st| lookup(NOUNS, st).or_else(|| lookup(VERBS, st)))
        {
            pieces.push(zh);
            hit += 1;
            total += 1;
            continue;
        }
        if let Some(zh) = lookup(VERBS, &l) {
            pieces.push(zh);
            hit += 1;
            total += 1;
            continue;
        }
        // 未命中：原样保留（**不猜**）
        pieces.push(t.clone());
        total += 1;
        unknown.push(t.clone());
    }
    (pieces, hit, total, unknown)
}

/// 生成**词干候选**（按可信度排序），由调用方用词典裁决 —— 函数本身不猜。
///
/// 顺序是刻意的：`s` 排在 `es` 前面，否则 `images` 会被砍成 `imag`（查不到），
/// 而正确词干是 `image`。反向的重辅音还原（`runn` → `run`）放在最后补位。
///
/// 刻意保守：短于 4 字符一律不处理（`ps` / `ls` 不能被砍成 `p` / `l`）。
fn stem_candidates(w: &str) -> Vec<String> {
    if w.len() < 4 || !w.chars().all(|c| c.is_ascii_alphabetic()) {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for suf in ["s", "es", "ing", "ed"] {
        if let Some(stem) = w.strip_suffix(suf) {
            if stem.len() >= 3 {
                out.push(stem.to_string());
            }
        }
    }
    // 重辅音还原：running → runn → run
    let extra: Vec<String> = out
        .iter()
        .filter(|s| {
            let b = s.as_bytes();
            b.len() >= 4 && b[b.len() - 1] == b[b.len() - 2]
        })
        .map(|s| s[..s.len() - 1].to_string())
        .collect();
    out.extend(extra);
    out
}

/// 拼片段：中文直接连；英文片段两侧留空格（让"哪部分没翻"肉眼可见）。
fn join_pieces(pieces: &[String]) -> String {
    let mut s = String::new();
    for p in pieces {
        if p.is_empty() {
            continue;
        }
        let ascii_only = p.chars().all(|c| c.is_ascii());
        if ascii_only {
            if !s.is_empty() && !s.ends_with(' ') {
                s.push(' ');
            }
            s.push_str(p);
            s.push(' ');
        } else {
            s.push_str(p);
        }
    }
    s.trim().to_string()
}

/// 翻译一句 help 描述。**纯函数，无 IO。**
pub fn translate(en: &str) -> ZhText {
    let clean = en.trim().trim_end_matches(['.', ';']).trim();
    if clean.is_empty() {
        return ZhText::fallback(en);
    }

    let toks: Vec<String> = clean
        .split_whitespace()
        .map(|t| {
            t.trim_matches(|c: char| matches!(c, ',' | ':' | '(' | ')' | '"' | '\''))
                .to_string()
        })
        .filter(|t| !t.is_empty())
        .collect();
    if toks.is_empty() {
        return ZhText::fallback(en);
    }

    // 首词必须是已知动词，否则不做句式模板（宁可不译，也不乱拼语序）
    let verb_zh = lookup(VERBS, &lower(&toks[0]));
    let Some(verb_zh) = verb_zh else {
        // 无动词：整句逐词 gloss（如 "The name of the container"）
        let (pieces, hit, total, unknown) = gloss(&toks);
        return ZhText {
            en: en.to_string(),
            zh: join_pieces(&pieces),
            coverage: ratio(hit, total),
            unknown,
            patterned: false,
        };
    };

    // 动词后的部分里找结构词（of 优先，其次 from，再次 to/into）
    let rest = &toks[1..];
    let pivot = rest.iter().enumerate().find_map(|(i, t)| {
        let l = lower(t);
        match l.as_str() {
            "of" => Some((i, Pivot::Of)),
            "from" => Some((i, Pivot::From)),
            "to" | "into" => Some((i, Pivot::To)),
            _ => None,
        }
    });

    let (pieces, hit, total, unknown) = match pivot {
        Some((i, p)) => {
            let head = &rest[..i];
            let tail = &rest[i + 1..];
            let (hp, hh, ht, hu) = gloss(head);
            let (tp, th, tt, tu) = gloss(tail);
            let head_txt = join_pieces(&hp);
            let tail_txt = join_pieces(&tp);
            let mut pieces = vec![verb_zh.clone()];
            match p {
                // `Show the history of an image` → 显示镜像的历史
                Pivot::Of => {
                    pieces.push(tail_txt);
                    pieces.push("的".into());
                    pieces.push(head_txt);
                }
                // `Build an image from a Dockerfile` → 从 Dockerfile 构建镜像
                Pivot::From => {
                    pieces = vec!["从".into(), tail_txt, verb_zh.clone(), head_txt];
                }
                // `Copy files to the host` → 把文件复制到主机
                Pivot::To => {
                    pieces = vec![
                        "把".into(),
                        head_txt,
                        verb_zh.clone(),
                        "到".into(),
                        tail_txt,
                    ];
                }
            }
            let mut unknown = hu;
            unknown.extend(tu);
            (pieces, hh + th, ht + tt, unknown)
        }
        None => {
            let (p, h, t, u) = gloss(rest);
            let mut pieces = vec![verb_zh.clone()];
            pieces.extend(p);
            (pieces, h + 1, t + 1, u)
        }
    };

    ZhText {
        en: en.to_string(),
        zh: join_pieces(&pieces),
        coverage: ratio(hit, total),
        unknown,
        patterned: true,
    }
}

fn ratio(hit: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        hit as f64 / total as f64
    }
}

/// 说明书条目 = 原动作 + 中文译文。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ZhAction {
    pub full_cmd: String,
    /// 可直接照抄的示例（有则优先，作为训练数据的 `cmd`）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    pub readonly: bool,
    pub desc_en: String,
    pub desc_zh: String,
    pub coverage: f64,
    pub unknown: Vec<String>,
}

pub fn translate_action(a: &HelpAction) -> ZhAction {
    let z = translate(&a.desc);
    ZhAction {
        full_cmd: a.full_cmd.clone(),
        example: a.example.clone(),
        readonly: a.readonly,
        desc_en: z.en,
        desc_zh: z.zh,
        coverage: z.coverage,
        unknown: z.unknown,
    }
}

/// 说明书统计（用于在文档里**如实**标注翻译质量）
#[derive(Debug, Clone, serde::Serialize)]
pub struct ManualStats {
    pub total: usize,
    pub readonly: usize,
    pub write: usize,
    /// 平均覆盖率
    pub avg_coverage: f64,
    /// 完全没翻出中文的条数（coverage == 0）
    pub untranslated: usize,
}

pub fn stats(acts: &[ZhAction]) -> ManualStats {
    let total = acts.len();
    let ro = acts.iter().filter(|a| a.readonly).count();
    let avg = if total == 0 {
        0.0
    } else {
        acts.iter().map(|a| a.coverage).sum::<f64>() / total as f64
    };
    ManualStats {
        total,
        readonly: ro,
        write: total - ro,
        avg_coverage: avg,
        untranslated: acts.iter().filter(|a| a.coverage <= 0.0).count(),
    }
}

/// 渲染中文说明书（Markdown）。**只读 / 写操作分组** —— 与 T1 门的判定对应。
pub fn render_manual(cli: &str, acts: &[ZhAction], help_len: usize) -> String {
    let s = stats(acts);
    let mut out = String::new();
    out.push_str(&format!("# {cli} 命令说明书（本地生成）\n\n"));
    out.push_str(&format!(
        "> 来源：`{cli} --help`（{help_len} 字符）→ 确定性解析 → 本地词典翻译。\n\
         > **未联网、未用模型**。译文里混排的英文是**词典未命中的原词**，不是笔误。\n\n"
    ));
    out.push_str(&format!(
        "| 条目 | 只读 | 写操作 | 平均翻译覆盖率 | 完全未译 |\n|---|---|---|---|---|\n| {} | {} | {} | {:.0}% | {} |\n\n",
        s.total,
        s.readonly,
        s.write,
        s.avg_coverage * 100.0,
        s.untranslated
    ));

    let section = |title: &str, only_read: bool, out: &mut String| {
        let rows: Vec<&ZhAction> = acts.iter().filter(|a| a.readonly == only_read).collect();
        if rows.is_empty() {
            return;
        }
        out.push_str(&format!("## {title}（{} 条）\n\n", rows.len()));
        out.push_str("| 命令 | 中文说明 | 英文原文 | 覆盖率 |\n|---|---|---|---|\n");
        for a in rows {
            let cmd = a.example.clone().unwrap_or_else(|| a.full_cmd.clone());
            out.push_str(&format!(
                "| `{cmd}` | {} | {} | {:.0}% |\n",
                esc(&a.desc_zh),
                esc(&a.desc_en),
                a.coverage * 100.0
            ));
        }
        out.push('\n');
    };
    section("只读（可放心执行）", true, &mut out);
    section("写操作（需确认）", false, &mut out);
    out
}

/// Markdown 表格转义：竖线会把单元格撑破
fn esc(s: &str) -> String {
    s.replace('|', "\\|")
}

/// 一条训练样本 —— **冷启动语料的单元**。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrainSample {
    /// 中文人话（**句子**，不是词表 —— 提示里出现孤立名词列表是 v18d/e 两次踩过的红线）
    pub nl: String,
    /// 命令（优先用可执行的具体示例）
    pub cmd: String,
    /// 模板形态（含占位符，供下游做槽位训练）
    pub template: String,
    pub cli: String,
    pub readonly: bool,
    /// 这条 NL 的翻译覆盖率 —— **下游过滤的依据**（insert 到训练数据前按它筛）
    pub coverage: f64,
    /// `action` = 正常动作；`reject` = 出域请求（**拒绝正样本**，我们 reject 一直是 0%）
    pub kind: String,
}

/// 从中文说明派生多条中文问法。
///
/// ⚠️ 两条纪律：
/// 1. 每条都必须是**完整中文句子**，且**不含命令本体**（否则模型会抄题面）；
/// 2. **不能用「X一下」模板** —— 实测它对动宾短语会生成病句
///    （"显示工作树状态一下" / "Clone a repository into a new directory 一下"）。
///    中文的「V一下 N」要求把动词拆出去重组，那属于生成，不是确定性改写，不做。
pub fn nl_variants(desc_zh: &str) -> Vec<String> {
    let d = desc_zh.trim();
    if d.is_empty() {
        return Vec::new();
    }
    vec![
        format!("{d}"),
        format!("帮我{d}"),
        format!("我想{d}"),
        format!("怎么{d}？"),
        format!("请{d}"),
    ]
}

/// 出域请求池 —— **拒绝能力的唯一正样本来源**。
///
/// 为什么要写死一份：实测 `reject = 0%`（底座与所有训练臂都不拒绝），
/// 而拒绝能力的唯一防线是 T1 门 / 判定式的"选 0"。要让模型学会"选 0"，
/// 训练集里**必须有"给不出候选"的样本**，否则它永远只会从候选中硬挑一个。
pub const REJECT_NLS: &[&str] = &[
    "帮我把这段话翻译成英文",
    "给我讲个笑话",
    "今天天气怎么样",
    "帮我订一张去北京的高铁票",
    "写一首关于夏天的诗",
    "你觉得我应该买哪只股票",
    "帮我预约明天下午的牙医",
    "我心情不好，陪我说说话",
    "推荐几部好看的电影",
    "帮我给老板写封辞职信",
    "算一下 1234 乘 5678 等于多少",
    "帮我订个外卖",
];

pub fn reject_samples(cli: &str) -> Vec<TrainSample> {
    REJECT_NLS
        .iter()
        .map(|nl| TrainSample {
            nl: (*nl).to_string(),
            cmd: String::new(),
            template: String::new(),
            cli: cli.to_string(),
            readonly: true,
            coverage: 1.0,
            kind: "reject".to_string(),
        })
        .collect()
}

/// 说明书 → 训练样本（每个合格动作 5 条中文问法，答案同一条命令）。
///
/// `min_coverage` 是**数据质量的门禁**：翻译覆盖率低于它的动作**直接不产出样本**。
///
/// 为什么要挡：这是我方全部负面实验的共同根因 —— v13 的合成语料里装的是"我们的想象"，
/// 而不是事实，最后被训进权重（`Δ_SFT = −43.3pp`）。半吊子译文
/// （"显示 working tree 状态" / "Join two 若干 development histories together"）
/// 与Hallucination只有一线之隔。**宁可少几千条，也不要几百条错的。**
pub fn to_samples(cli: &str, acts: &[ZhAction], min_coverage: f64) -> Vec<TrainSample> {
    let mut out = Vec::new();
    for a in acts {
        if a.coverage < min_coverage {
            continue;
        }
        let cmd = a.example.clone().unwrap_or_else(|| a.full_cmd.clone());
        for nl in nl_variants(&a.desc_zh) {
            out.push(TrainSample {
                nl,
                cmd: cmd.clone(),
                template: a.full_cmd.clone(),
                cli: cli.to_string(),
                readonly: a.readonly,
                coverage: a.coverage,
                kind: "action".to_string(),
            });
        }
    }
    out
}

/// 有多少动作**够格**进训练数据（用于如实报告丢弃量）
pub fn eligible(acts: &[ZhAction], min_coverage: f64) -> usize {
    acts.iter().filter(|a| a.coverage >= min_coverage).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verb_plus_noun_is_translated() {
        let z = translate("List containers");
        assert_eq!(z.zh, "列出容器");
        assert!((z.coverage - 1.0).abs() < 1e-9, "coverage={}", z.coverage);
        assert!(z.unknown.is_empty());
    }

    #[test]
    fn of_pivot_reorders_to_chinese_word_order() {
        // 英文 `X of Y` → 中文 `Y的X`
        let z = translate("Show the history of an image");
        assert_eq!(z.zh, "显示镜像的历史");
        assert!(z.patterned);
    }

    #[test]
    fn from_pivot_is_rendered_as_cong() {
        let z = translate("Build an image from a Dockerfile");
        assert!(z.zh.starts_with("从"), "zh={}", z.zh);
        assert!(z.zh.contains("构建"));
        assert!(z.zh.contains("镜像"));
    }

    #[test]
    fn to_pivot_is_rendered_as_ba() {
        let z = translate("Copy files to the host");
        assert!(z.zh.starts_with("把"), "zh={}", z.zh);
        assert!(z.zh.contains("到"));
    }

    #[test]
    fn unknown_words_are_kept_verbatim_not_guessed() {
        // `florble` 不在词典 → 必须原样保留，并出现在 unknown 里
        let z = translate("List florble widgets");
        assert!(z.zh.contains("florble"), "未命中词必须原样保留: {}", z.zh);
        assert!(z.unknown.iter().any(|u| u == "florble"));
        assert!(z.coverage < 1.0);
    }

    #[test]
    fn sentence_without_known_verb_degrades_to_gloss() {
        // 无动词开头 → 不做句式模板，逐词 gloss（`patterned == false`）
        let z = translate("The name of the container");
        assert!(!z.patterned);
        assert!(z.zh.contains("容器"), "zh={}", z.zh);
    }

    #[test]
    fn empty_and_punctuation_only_input_is_safe() {
        assert_eq!(translate("").coverage, 0.0);
        assert_eq!(translate("   ").zh, "   ");
        assert_eq!(translate("...").coverage, 0.0);
    }

    #[test]
    fn plurals_and_gerunds_hit_after_stemming() {
        // containers → container；running → run
        let z = translate("List running containers");
        assert!(z.zh.contains("容器"), "zh={}", z.zh);
        assert!(z.zh.contains("运行中的"), "zh={}", z.zh);
    }

    #[test]
    fn stemming_handles_plurals_and_gerunds_without_mangling_short_tokens() {
        // `ps`/`ls` 只有 2 字符 → 不产生词干，避免被砍成 `p`/`l`
        assert!(stem_candidates("ps").is_empty());
        assert!(stem_candidates("ls").is_empty());
        // images → image，且**去 s 必须排在去 es 之前**，否则得到 imag（查不到）
        let c = stem_candidates("images");
        assert_eq!(
            c[0], "image",
            "去 s 必须优先，否则 images→imag 命中不了词典"
        );
        // running → run（叠辅音还原）
        assert!(stem_candidates("running").contains(&"run".to_string()));
    }

    #[test]
    fn coverage_is_computed_against_content_words_only() {
        // `a`/`the` 属于 DROP，不计入分母；`log`/`container` 计入且都命中
        let z = translate("Fetch the logs of a container");
        assert!(
            (z.coverage - 1.0).abs() < 1e-9,
            "zh={} cov={}",
            z.zh,
            z.coverage
        );
    }

    #[test]
    fn nl_variants_are_full_sentences_not_word_lists() {
        let vs = nl_variants("列出容器");
        assert_eq!(vs.len(), 5);
        for v in &vs {
            assert!(v.contains("列出容器"), "变体必须包含完整说明: {v}");
            assert!(v.split_whitespace().count() <= 3, "不该长成一段话: {v}");
            assert!(!v.ends_with("一下"), "「X一下」对动宾短语会生成病句: {v}");
        }
    }

    #[test]
    fn low_coverage_actions_are_excluded_from_training_data() {
        let acts = vec![
            ZhAction {
                full_cmd: "docker ps".into(),
                example: Some("docker ps".into()),
                readonly: true,
                desc_en: "List containers".into(),
                desc_zh: "列出容器".into(),
                coverage: 1.0,
                unknown: vec![],
            },
            ZhAction {
                full_cmd: "docker xyzzy".into(),
                example: Some("docker xyzzy".into()),
                readonly: false,
                desc_en: "Xyzzy the florble".into(),
                desc_zh: "Xyzzy florble".into(),
                coverage: 0.0,
                unknown: vec!["Xyzzy".into(), "florble".into()],
            },
        ];
        // 默认门槛下，半吊子译文（coverage 0）**一条都不许进训练集**
        let s = to_samples("docker", &acts, 0.8);
        assert_eq!(s.len(), 5, "只应产出高覆盖动作的 5 条样本");
        assert!(s.iter().all(|x| x.cmd == "docker ps"));
        assert!(s.iter().all(|x| x.coverage >= 0.8));
        assert_eq!(eligible(&acts, 0.8), 1);
        // 门槛放到 0 时全部放行 —— 门禁必须真的受参数控制
        assert_eq!(to_samples("docker", &acts, 0.0).len(), 10);
    }

    #[test]
    fn samples_cover_every_action_and_include_rejects() {
        let acts = vec![ZhAction {
            full_cmd: "docker ps".into(),
            example: Some("docker ps".into()),
            readonly: true,
            desc_en: "List containers".into(),
            desc_zh: "列出容器".into(),
            coverage: 1.0,
            unknown: vec![],
        }];
        let s = to_samples("docker", &acts, 0.0);
        assert_eq!(s.len(), 5);
        assert!(s.iter().all(|x| x.cmd == "docker ps"));
        assert!(s.iter().all(|x| x.kind == "action"));

        let r = reject_samples("docker");
        assert!(!r.is_empty());
        assert!(r.iter().all(|x| x.kind == "reject" && x.cmd.is_empty()));
    }

    #[test]
    fn manual_marks_untranslated_counts_honestly() {
        let acts = vec![
            ZhAction {
                full_cmd: "docker ps".into(),
                example: None,
                readonly: true,
                desc_en: "List containers".into(),
                desc_zh: "列出容器".into(),
                coverage: 1.0,
                unknown: vec![],
            },
            ZhAction {
                full_cmd: "docker xyzzy".into(),
                example: None,
                readonly: false,
                desc_en: "Xyzzy the florble".into(),
                desc_zh: "Xyzzy florble".into(),
                coverage: 0.0,
                unknown: vec!["Xyzzy".into(), "florble".into()],
            },
        ];
        let md = render_manual("docker", &acts, 1234);
        assert!(md.contains("只读"), "必须有只读分组");
        assert!(md.contains("写操作"), "必须有写操作分组");
        assert!(md.contains("完全未译"), "必须如实标注未译条数");
        let st = stats(&acts);
        assert_eq!(st.untranslated, 1);
        assert_eq!(st.readonly, 1);
    }

    #[test]
    fn markdown_pipe_is_escaped_so_tables_survive() {
        let acts = vec![ZhAction {
            full_cmd: "c x".into(),
            example: None,
            readonly: true,
            desc_en: "a | b".into(),
            desc_zh: "甲 | 乙".into(),
            coverage: 1.0,
            unknown: vec![],
        }];
        let md = render_manual("c", &acts, 1);
        assert!(md.contains("\\|"), "竖线必须转义: {md}");
    }
}
