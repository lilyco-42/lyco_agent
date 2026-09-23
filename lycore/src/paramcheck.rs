//! 必需参数校验 —— T1 门的第二道闸（P1.2, 2026-09-21）
//!
//! ## 为什么必须存在（v18 实测失败模式 F4）
//!
//! 模型给出的命令**语法合法、风险等级也过门**，但**缺少必需参数**：
//! ```text
//! 用户: 装个 express
//! 模型: npm install          ← 语法合法、Write 级、确认后放行
//! 实际: 在**没有依赖的目录**里空跑, 或重装 node_modules —— 静默失效
//! ```
//!
//! 这类失败**比危险命令更隐蔽**：危险命令会被 `t1gate::classify` 拦下并吱声，
//! 空参命令却一路绿灯跑完、退出码 0、用户以为成了。
//!
//! ## 与 t1gate 的分工
//!
//! | 模块 | 回答的问题 | 失败时的表现 |
//! |---|---|---|
//! | `t1gate::classify` | **这条命令危险吗** | 拦截并说明原因 |
//! | `paramcheck::check`（本模块） | **这条命令能干活吗** | 判为 `NeedsParam`，提示缺什么 |
//!
//! 两者**正交**：`npm install` 不危险但缺参；`rm -rf /` 危险但不缺参；
//! `npm install express` 既安全也不缺参。故本模块**不重复**判定风险。
//!
//! ## 设计原则（照抄 t1gate 的教训）
//!
//! 1. **动词表驱动，不按程序硬编码** —— 对训练时未见过的 CLI 同样成立。
//! 2. **只报「确定缺」的，不猜** —— 宁漏勿错。误报会拦掉合法命令（把好命令挡了
//!    比放过坏命令更伤产品），故只覆盖有**强语法约定**的动词。
//! 3. **不依赖模型** —— 纯确定性规则，可单测、可复现。

/// 校验结论
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum Status {
    /// 参数齐备
    Ok,
    /// 缺必需参数（命令能跑但干不成活 = 静默失效）
    NeedsParam,
    /// 规则不覆盖该命令 —— **不代表没问题**，仅表示本模块不发表意见
    Unchecked,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::NeedsParam => "needs_param",
            Status::Unchecked => "unchecked",
        }
    }

    pub fn is_needs_param(&self) -> bool {
        matches!(self, Status::NeedsParam)
    }
}

/// 校验结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct ParamCheck {
    pub status: Status,
    /// 缺失的槽位名（给用户看的占位符, 如 `<pkg>`）
    pub missing: Vec<String>,
    /// 命中的动词（便于排障：为什么判它缺参）
    pub matched: Option<String>,
    /// 给用户的补全提示
    pub reason: String,
}

impl ParamCheck {
    fn ok(verb: &str) -> Self {
        Self {
            status: Status::Ok,
            missing: vec![],
            matched: Some(verb.to_string()),
            reason: String::new(),
        }
    }

    fn unchecked(verb: &str) -> Self {
        Self {
            status: Status::Unchecked,
            missing: vec![],
            matched: Some(verb.to_string()),
            reason: format!("`{verb}` 未纳入必需参数规则，本模块不发表意见"),
        }
    }

    fn missing(verb: &str, slots: &[&str]) -> Self {
        let missing: Vec<String> = slots.iter().map(|s| (*s).to_string()).collect();
        Self {
            status: Status::NeedsParam,
            reason: format!(
                "`{verb}` 缺少必需参数 {}，空跑等于静默失效",
                missing.join(" ")
            ),
            missing,
            matched: Some(verb.to_string()),
        }
    }
}

/// **必需参数规则表** —— 每条：(程序名, 动词, 必需槽位, 可豁免的 flag)
///
/// 只收录**语义上必须有实参、否则命令无意义**的组合。判定时看「除程序名/动词/flag
/// 之外还有没有剩余 token」——没有就是缺参。
///
/// 收录标准（宁漏勿错）：
/// - `npm install` / `pnpm add` / `cargo add` → 必须有包名
/// - `git remote add` → 必须有名 + URL
/// - `docker run` → 必须有镜像名
/// - `curl` / `wget` → 必须有 URL
/// - `kill` / `pkill` → 必须有目标（`-l` 列信号除外）
const RULES: &[(&str, &str, &[&str], &[&str])] = &[
    // 包管理：装东西必须说清装什么
    (
        "npm",
        "install",
        &["<pkg>"],
        &["-g", "--global", "-D", "--save-dev", "-S", "--save"],
    ),
    ("npm", "i", &["<pkg>"], &["-g", "--global"]),
    ("npm", "uninstall", &["<pkg>"], &["-g", "--global"]),
    ("npm", "rm", &["<pkg>"], &["-g", "--global"]),
    ("pnpm", "add", &["<pkg>"], &["-g", "--global", "-D"]),
    ("pnpm", "install", &["<pkg>"], &["-g"]),
    ("yarn", "add", &["<pkg>"], &["-D", "--dev"]),
    (
        "pip",
        "install",
        &["<pkg>"],
        &["-r", "--requirement", "-U", "--upgrade"],
    ),
    ("pip3", "install", &["<pkg>"], &["-r", "--requirement"]),
    ("cargo", "add", &["<crate>"], &["--dev", "--build"]),
    ("cargo", "install", &["<crate>"], &["--git", "--path"]),
    ("apt", "install", &["<pkg>"], &["-y", "--yes"]),
    ("apt-get", "install", &["<pkg>"], &["-y", "--yes"]),
    ("go", "get", &["<pkg>"], &[]),
    // git：需要对象的写操作（两级动词用 "a b" 形式）
    ("git", "remote add", &["<name>", "<url>"], &[]),
    ("git", "checkout", &["<branch>"], &["-b", "-B"]),
    (
        "git",
        "branch",
        &["<branch>"],
        &["-a", "-r", "-d", "-D", "-m", "-l", "--list", "-v"],
    ),
    ("git", "clone", &["<repo>"], &[]),
    ("git", "tag", &["<tag>"], &["-l", "--list", "-d"]),
    // 容器：跑容器必须说跑什么镜像
    ("docker", "run", &["<image>"], &["-i", "--interactive"]),
    ("docker", "pull", &["<image>"], &[]),
    ("docker", "push", &["<image>"], &[]),
    (
        "docker",
        "exec",
        &["<container>", "<cmd>"],
        &["-i", "-t", "-it"],
    ),
    ("docker", "build", &["<context>"], &["-t", "--tag"]),
    ("kubectl", "exec", &["<pod>"], &["-i", "-t", "-it"]),
    // 网络：请求必须有目标
    ("curl", "", &["<url>"], &["-V", "--version", "-h", "--help"]),
    ("wget", "", &["<url>"], &["-V", "--version", "-h", "--help"]),
    // 进程：必须说杀谁
    ("kill", "", &["<pid>"], &["-l", "-L", "-s"]),
    ("pkill", "", &["<pattern>"], &["-l", "-L"]),
    // FFmpeg：必须说输入（-i）
    (
        "ffmpeg",
        "",
        &["-i <input>"],
        &["-version", "-h", "--help", "-formats", "-codecs"],
    ),
];

/// 判断一个 token 是否像 flag（`-x` / `--xyz`）
fn is_flag(t: &str) -> bool {
    t.starts_with('-') && t.len() > 1
}

/// 需要「吃下一个 token」的 flag（如 `-t mytag`、`-m msg`、`-C path`）
/// 用于把 flag 的值排除出「剩余实参」的统计。
fn takes_value(flag: &str) -> bool {
    matches!(
        flag,
        "-t" | "--tag"
            | "-m"
            | "--message"
            | "-C"
            | "--cwd"
            | "-o"
            | "--output"
            | "-b"
            | "-B"
            | "-c"
            | "--config"
            | "-p"
            | "--port"
            | "-n"
            | "--name"
            | "-u"
            | "--user"
            | "-e"
            | "--env"
            | "-v"
            | "--volume"
            | "--git"
            | "--path"
    )
}

/// 校验一条命令的必需参数。
///
/// **只看单条命令**：组合命令（`&&` / `;` / `|`）请调用方拆开逐条校验
/// （与 `t1gate::classify` 的拆分口径一致，见该模块的 `parts` 逻辑）。
pub fn check(raw_cmd: &str) -> ParamCheck {
    let cmd = crate::t1gate::normalize(raw_cmd);
    if cmd.is_empty() || cmd.contains("无需调用") {
        return ParamCheck::unchecked("");
    }

    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    let Some(prog_raw) = tokens.first() else {
        return ParamCheck::unchecked("");
    };
    let prog = prog_raw.to_ascii_lowercase();
    // 跳过 env 前缀（`sudo npm install` / `FOO=1 cmd`）。
    // `cmd_idx` = 真程序名在 tokens 里的下标；动词紧随其后。
    let (prog, cmd_idx) = match prog.as_str() {
        "sudo" | "env" | "time" | "nohup" => match tokens.get(1) {
            Some(t) => (t.to_ascii_lowercase(), 1),
            None => return ParamCheck::unchecked(&prog),
        },
        _ => (prog, 0),
    };
    let verb_idx = cmd_idx + 1;

    // 匹配规则：程序名 + 动词。动词可含多词（`git remote add`）；动词为 "" 时只匹配程序名。
    let rule = RULES.iter().find(|(p, v, _, _)| {
        if *p != prog {
            return false;
        }
        if v.is_empty() {
            return true;
        }
        let words: Vec<&str> = v.split_whitespace().collect();
        words.iter().enumerate().all(|(i, w)| {
            tokens
                .get(verb_idx + i)
                .is_some_and(|t| t.eq_ignore_ascii_case(w))
        })
    });
    let Some((_, verb, slots, exempt)) = rule else {
        return ParamCheck::unchecked(&prog);
    };
    let verb_label = if verb.is_empty() { prog.as_str() } else { verb };

    // 统计「剩余实参」：从动词之后开始，跳过 flag、以及 flag 的值。
    // ⚠️ 动词可含多词（`git remote add`）→ 按空格数推进起始下标。
    let verb_words = if verb.is_empty() {
        0
    } else {
        verb.split_whitespace().count()
    };
    let start = verb_idx + verb_words;
    let mut operands: Vec<&str> = Vec::new();
    let mut skip_next = false;
    for t in tokens.iter().skip(start) {
        let t_trim = t.trim_matches(|c| c == '"' || c == '\'');
        if skip_next {
            skip_next = false;
            continue;
        }
        if is_flag(t_trim) {
            // ⚠️ 豁免 flag 若**会吃值**（`cargo add --git <url>` / `docker build -t <tag>`），
            // 仍须吞掉它的值 —— 否则 tag/url 会被误算成 operand，导致缺参漏判。
            if takes_value(t_trim) {
                skip_next = true;
            }
            continue;
        }
        operands.push(t_trim);
    }

    // 信息类调用豁免：**仅对「动词为空」的程序生效**（`curl --version` / `ffmpeg -h`）。
    // 判定方式：命令的参数部分**只由豁免 flag 构成**。
    // ⚠️ 有动词的命令（`git checkout -b`）不适用 —— 那是**flag 缺值**，必须报缺参。
    let only_exempt_flags = verb.is_empty()
        && tokens.iter().skip(start).any(|t| !t.is_empty())
        && tokens
            .iter()
            .skip(start)
            .all(|t| exempt.contains(&t.trim_matches(|c| c == '"' || c == '\'')));
    if only_exempt_flags {
        return ParamCheck::ok(verb_label);
    }

    if operands.is_empty() {
        return ParamCheck::missing(verb_label, slots);
    }

    // 槽位数量够不够（如 `git remote add foo` 缺 URL）
    if operands.len() < slots.len() {
        let missing: Vec<&str> = slots.iter().skip(operands.len()).copied().collect();
        return ParamCheck::missing(verb_label, &missing);
    }
    ParamCheck::ok(verb_label)
}

/// 是否为「缺参」判定 —— 供门控链快速询问
pub fn needs_param(raw_cmd: &str) -> bool {
    check(raw_cmd).status.is_needs_param()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 核心动机用例：v18 F4 实测的失败模式
    #[test]
    fn f4_missing_pkg_is_caught() {
        let c = check("npm install");
        assert_eq!(c.status, Status::NeedsParam, "got={c:?}");
        assert_eq!(c.missing, vec!["<pkg>"]);
        assert!(needs_param("npm install"));
    }

    /// 补齐参数后必须放行（否则就是在拦合法命令）
    #[test]
    fn completed_command_passes() {
        for c in [
            "npm install express",
            "npm install express --save-dev",
            "npm i lodash",
            "pnpm add zod",
            "cargo add serde",
            "pip install requests",
            "yarn add react",
            "go get github.com/x/y",
            "git clone https://github.com/a/b",
            "docker run -it ubuntu",
            "docker pull nginx:latest",
            "curl https://example.com",
            "wget http://x.com/f.tar.gz",
            "kill 1234",
            "pkill nginx",
            "ffmpeg -i in.mp4 out.mp4",
        ] {
            let c = check(c);
            assert_ne!(c.status, Status::NeedsParam, "{c:?} 不该判缺参");
        }
    }

    /// 空参必须被抓
    #[test]
    fn bare_verbs_are_caught() {
        for c in [
            "npm install",
            "npm uninstall",
            "pip install",
            "cargo add",
            "pnpm add",
            "yarn add",
            "docker run",
            "docker pull",
            "git clone",
            "curl",
            "wget",
            "kill",
            "pkill",
            "ffmpeg",
        ] {
            assert!(needs_param(c), "{c} 应判缺参");
        }
    }

    /// 多槽位：`git remote add` 缺 URL（只给了名字）
    #[test]
    fn multi_slot_partial_is_caught() {
        let c = check("git remote add origin");
        assert_eq!(c.status, Status::NeedsParam, "got={c:?}");
        assert!(
            c.missing.contains(&"<url>".to_string()),
            "missing={:?}",
            c.missing
        );
        // 补全后放行
        let ok = check("git remote add origin https://github.com/a/b");
        assert_eq!(ok.status, Status::Ok);
    }

    /// 豁免 flag 不得被当成"实参"（否则 `docker build -t foo` 会误判为有 context）
    #[test]
    fn flags_do_not_count_as_operands() {
        // `docker build` 缺 context（. 也没给）
        assert!(needs_param("docker build -t myimg"));
        // 给上 context 就放行
        assert_eq!(check("docker build -t myimg .").status, Status::Ok);
        // flag 的值也不能当实参
        assert!(needs_param("git checkout -b"));
    }

    /// 冒险区边界：不能误伤
    #[test]
    fn must_not_false_positive() {
        for c in [
            "gh issue list",
            "git status",
            "git log --oneline",
            "docker ps",
            "kubectl get nodes",
            "cargo check",
            "npm run build",
            "git add .",
            "ls -la",
            // 未纳入规则的程序 → Unchecked（不拦）
            "terraform plan",
            "jj log",
            "docker system prune -a",
        ] {
            assert_ne!(
                check(c).status,
                Status::NeedsParam,
                "{c} 被误判为缺参: {:?}",
                check(c)
            );
        }
    }

    /// 未覆盖的程序必须诚实报 Unchecked，而非假装 OK
    #[test]
    fn uncovered_reports_unchecked_not_ok() {
        assert_eq!(check("terraform plan").status, Status::Unchecked);
        assert_eq!(check("jj log").status, Status::Unchecked);
        // 但已覆盖且齐备的应报 Ok
        assert_eq!(check("npm install express").status, Status::Ok);
    }

    /// 剥 brush 前缀后照样判定（模型实际输出形态）
    #[test]
    fn brush_prefix_is_normalized() {
        assert!(needs_param("brush npm install"));
        assert_eq!(check("brush npm install express").status, Status::Ok);
    }

    /// sudo / 环境变量前缀
    #[test]
    fn sudo_prefix_handled() {
        assert!(needs_param("sudo npm install"));
        assert_eq!(check("sudo npm install express").status, Status::Ok);
        assert!(needs_param("sudo apt install"));
        assert_eq!(check("sudo apt install nginx").status, Status::Ok);
    }

    /// 信息类调用不算缺参（`curl --version` / `ffmpeg -h` 是合法命令）
    #[test]
    fn informational_flags_are_exempt() {
        for c in [
            "curl --version",
            "curl -V",
            "ffmpeg -version",
            "ffmpeg -h",
            "wget -V",
        ] {
            let r = check(c);
            assert_ne!(
                r.status,
                Status::NeedsParam,
                "{c} 是合法信息调用, 不该判缺参: {r:?}"
            );
        }
        // 但没有目标 URL 的裸 `curl` 仍必须判缺参
        assert!(needs_param("curl"));
        assert!(needs_param("curl -s"));
    }
}
