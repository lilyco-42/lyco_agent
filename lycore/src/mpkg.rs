//! mpkg 记忆包 —— **格式层**（content-id / 清单校验 / kbv → 包目录胶水）。
//!
//! 定位：`lyco_agent` 从视频/手册学到的知识要能打包成**可回放、可验证、content-addressed**
//! 的记忆原子（见 `lystack/proto/mpkg/GOLDEN.md`）。此前 `mpkg.json` 是手写的
//! （`docs/mvp-mpkg-loop-2026-09-16.md` §四.3 记的待补项），本模块把它固化成 CLI。
//!
//! **契约对齐（GOLDEN.md §6：新实现验收标准）**：本文件是契约源的第 3 个实现
//! （前两个：lilyco `lilyco-mpkg/src/pack.rs`、cache-node `src/mpkg_verify.rs`）。
//! 自带 golden 三例测试（canon 幂等 + content-id 逐字节），与注册表侧
//! `mpkg-registry/tools/reindex.py` 的同一批向量**互证** —— 两边都过 = 格式没漂。
//!
//! 两条铁律（踩过才知道）：
//! 1. **两层哈希别混用**：content-id 对 `canon_json(包JSON)` 取哈希（对序列化空白不敏感）；
//!    `files` 表里的哈希是对**文件原始字节**取（每个字节都敏感，`mpkg.json` 自己被包含在内）。
//! 2. **canon = 递归键排序 + 紧凑分隔符 + UTF-8 原样**。serde_json 的 `Map` 默认是
//!    `BTreeMap`（键序即序列化序），`Value::to_string()` 即紧凑形态，恰好等价于
//!    Python 的 `json.dumps(sort_keys=True, separators=(',',':'), ensure_ascii=False)`。
//!    **清单里避免浮点**：Rust/Python 的浮点字面量格式并非处处一致（`1e-5` vs `1e-05`）。

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// 格式版本（GOLDEN.md §1：顶层只允许 `manifest` 与 `files`；清单必需六字段）。
pub const FORMAT: &str = "0.1";

/// 清单文件名（包目录里的唯一必需文件）。
pub const MANIFEST: &str = "mpkg.json";

// ── sha256（自实现：lycore 保持零多余依赖，与 lilyco-mpkg 的 sha256.rs 同策略） ──

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// SHA-256 → 64 位小写 hex。
///
/// 为什么自实现而不是拉 `sha2` crate：lycore 要交叉编译到 Android/aarch64（AGENTS.md
/// 的硬约束是依赖面越小越好），而这里只需一个函数；正确性由 NIST 向量 + golden 三例
/// 双重钉住（算法错 → 全部测试同时红）。
#[allow(clippy::needless_range_loop)] // 下标同时索引 w/K 两个数组，迭代器写法更晦涩
fn sha256_hex(data: &[u8]) -> String {
    let mut h = H0;
    let bit_len = (data.len() as u64) * 8;
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let x = w[i - 15];
            let y = w[i - 2];
            let s0 = x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3);
            let s1 = y.rotate_right(17) ^ y.rotate_right(19) ^ (y >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (i, v) in [a, b, c, d, e, f, g, hh].iter().enumerate() {
            h[i] = h[i].wrapping_add(*v);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

// ── canon / content-id ────────────────────────────────────────────────────────

/// canon_json：递归键排序 + 紧凑分隔符 + UTF-8 原样（GOLDEN.md §2）。
///
/// 依赖 serde_json 的默认行为：`Value::Object` 的 `Map` = `BTreeMap`（无 `preserve_order`），
/// 且 `Value::to_string()` 走紧凑序列化器、非 ASCII 不转义。
pub fn canon(v: &Value) -> String {
    v.to_string()
}

/// content-id = `sha256:` + sha256(canon_json(包JSON) 的 UTF-8 字节)。
pub fn content_id(pack: &Value) -> String {
    format!("sha256:{}", sha256_hex(canon(pack).as_bytes()))
}

// ── 清单校验（比 mpkg.py 略严：name 强制 kebab-case，防路径穿越） ──────────────

/// 校验清单。返错即拒（宁缺毋滥：解析出垃圾比解析出 0 条更危险）。
pub fn validate(m: &Value) -> Result<()> {
    for k in ["mpkg", "name", "version", "intent", "steps", "verify"] {
        if m.get(k).is_none() {
            bail!("{MANIFEST} 缺必需字段: {k}");
        }
    }
    if m.get("mpkg").and_then(Value::as_str) != Some(FORMAT) {
        bail!("不支持的 mpkg 版本（期望 {FORMAT}）");
    }
    let name = m.get("name").and_then(Value::as_str).unwrap_or("");
    if name.is_empty()
        || name.starts_with('-')
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        bail!("name 必须为小写 kebab-case（只允许 [a-z0-9-]）: {name:?}");
    }
    for k in ["version", "intent"] {
        if m.get(k).and_then(Value::as_str).unwrap_or("").is_empty() {
            bail!("{k} 必须为非空字符串");
        }
    }
    match m.get("steps").and_then(Value::as_array) {
        Some(s) if !s.is_empty() => {
            for (i, step) in s.iter().enumerate() {
                let ok = step
                    .get("run")
                    .and_then(Value::as_str)
                    .is_some_and(|r| !r.is_empty());
                if !ok {
                    bail!("steps[{i}] 必须含非空字符串 run");
                }
            }
        }
        _ => bail!("steps 必须为非空数组（只允许真实 CLI 进程）"),
    }
    match m.get("verify").and_then(Value::as_array) {
        Some(v) if !v.is_empty() => {
            for (i, cmd) in v.iter().enumerate() {
                if !cmd.as_str().is_some_and(|c| !c.is_empty()) {
                    bail!("verify[{i}] 必须为非空字符串");
                }
            }
        }
        _ => bail!("verify 必须为非空数组"),
    }
    Ok(())
}

/// 读包目录里的 mpkg.json 并校验。
pub fn load_manifest(dir: &Path) -> Result<Value> {
    let path = dir.join(MANIFEST);
    let raw = fs::read_to_string(&path).with_context(|| format!("读不到 {}", path.display()))?;
    let m: Value =
        serde_json::from_str(&raw).with_context(|| format!("{} 不是合法 JSON", path.display()))?;
    validate(&m)?;
    Ok(m)
}

/// 收集包内文件 → `{相对路径: sha256 hex}`。口径与 `mpkg.py collect_files` 一致：
/// 跳过 `dist/`、`__pycache__/`、以 `.` 开头的目录、以及 `*.mpkg` 本体；路径分隔符统一 `/`。
///
/// 注意：`mpkg.json` **自己也在表里** —— 它的字节被 content-id 间接锁定（改一个空格
/// 就是另一个包），这是有意的。
pub fn collect_files(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir)
            .with_context(|| format!("读目录失败 {}", dir.display()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let ft = e.file_type()?;
            if ft.is_dir() {
                if name.starts_with('.') || name == "dist" || name == "__pycache__" {
                    continue;
                }
                stack.push(path);
            } else if ft.is_file() {
                if name.ends_with(".mpkg") {
                    continue;
                }
                let rel = path
                    .strip_prefix(root)
                    .map_err(|e| anyhow!("相对路径失败 {}: {e}", path.display()))?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, sha256_hex(&fs::read(&path)?));
            }
        }
    }
    Ok(out)
}

/// 包目录 → (清单, content-id, 文件数)。
pub fn package_id_of_dir(dir: &Path) -> Result<(Value, String, usize)> {
    let manifest = load_manifest(dir)?;
    let files = collect_files(dir)?;
    if files.is_empty() {
        bail!("包目录里没有文件: {}", dir.display());
    }
    let id = content_id(&json!({ "manifest": &manifest, "files": &files }));
    Ok((manifest, id, files.len()))
}

// ── verify（回放执行，T3 动作）─────────────────────────────────────────────────
//
// 语义与契约参考实现 lystack proto/py/mpkg.py 的 cmd_verify 严格对齐：
//   - steps 与 verify 都以 **work**（临时目录）为 cwd；`{{pkg}}` = 包目录本身
//     （dir 形态文件已在位，无需解包）、`{{work}}` = 临时工作区；
//   - POSIX shell（bash/sh 顺位）；`requirements.tools[].name` 逐个 which，缺即拒；
//   - step 的 expect.exit 缺省 0（validate 不校验 expect 形状，漂移表 §5.6）；
//   - 单命令超时：Python subprocess 有 timeout，cache-node 版只声明未实现 ——
//     本实现用 try_wait 轮询真做（agent 运行时执行任意清单命令，无超时=自杀）。
// attestation 与 Python 版同构（replayed_at 用 epoch 秒，cache-node 同款，零依赖）。

/// 单条命令的回放超时（秒）。
pub const STEP_TIMEOUT_SECS: u64 = 120;

/// 沿 PATH 找可执行体（Windows 补 .exe/.cmd/.bat 后缀）。
fn which(exe: &str) -> Option<String> {
    let exts: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        for ext in exts {
            let p = dir.join(format!("{exe}{ext}"));
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn find_shell() -> Result<String> {
    ["bash", "sh"]
        .iter()
        .find_map(|sh| which(sh))
        .map(|p| p.replace('\\', "/"))
        .ok_or_else(|| {
            anyhow!("回放需要 POSIX shell（bash/sh）；Windows 请装 Git Bash 或把它加进 PATH")
        })
}

fn subst(cmd: &str, pkg: &str, work: &str) -> String {
    cmd.replace("{{pkg}}", pkg).replace("{{work}}", work)
}

/// 尾部截断（字符级，不劈 UTF-8），与 Python 版 `[-400:]` 口径一致。
fn tail400(s: &str) -> String {
    let n = s.chars().count();
    if n <= 400 {
        s.to_string()
    } else {
        s.chars().skip(n - 400).collect()
    }
}

/// 排空管道到字符串（读线程；不排空会在子进程写满管道时死锁）。
fn drain_pipe<R: std::io::Read + Send + 'static>(mut pipe: Option<R>) -> String {
    let mut buf = Vec::new();
    if let Some(p) = pipe.as_mut() {
        let _ = p.read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).trim_end().to_string()
}

/// 跑一条 shell 命令：stdout/stderr 各自读线程接走，主线程 try_wait 轮询 +
/// 截止时间击杀。返回 (exit, stdout, stderr, 耗时ms)。
fn run_timed(
    shell: &str,
    cmd: &str,
    work: &Path,
    timeout: std::time::Duration,
) -> Result<(i32, String, String, u128)> {
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Instant;

    let t0 = Instant::now();
    let mut child = Command::new(shell)
        .arg("-c")
        .arg(cmd)
        .current_dir(work)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn 失败: {cmd}"))?;

    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();
    let th_out = thread::spawn(move || drain_pipe(out_pipe));
    let th_err = thread::spawn(move || drain_pipe(err_pipe));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(anyhow!("执行超时（>{}s）: {cmd}", timeout.as_secs()));
                }
                thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => return Err(anyhow!("wait 失败: {e}")),
        }
    };
    let code = status.code().unwrap_or(-1);
    let so = th_out.join().unwrap_or_default();
    let se = th_err.join().unwrap_or_default();
    Ok((code, so, se, t0.elapsed().as_millis()))
}

/// 回放包目录：清单校验 → content-id 复算 → 工具需求 → steps → verify → attestation。
/// attestation 形状与 lystack proto/py/mpkg.py 同构；回放失败（ok=false）**不报 Err**
/// —— 回放结论是正常产物，程序性错误（读不到包/缺工具/无 shell）才 Err，
/// 退出码语义由 CLI 层决定。
pub fn verify_dir(dir: &Path) -> Result<Value> {
    verify_dir_with(dir, std::time::Duration::from_secs(STEP_TIMEOUT_SECS))
}

pub fn verify_dir_with(dir: &Path, timeout: std::time::Duration) -> Result<Value> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let (manifest, id, n_files) = package_id_of_dir(dir)?;
    let pkg = dir
        .canonicalize()
        .with_context(|| format!("canonicalize 失败 {}", dir.display()))?
        .to_string_lossy()
        .replace('\\', "/");

    // 工具需求：缺一个就整个包拒绝回放（Python 原版同款，宁可不做不做半截）。
    let mut missing: Vec<String> = Vec::new();
    if let Some(tools) = manifest
        .get("requirements")
        .and_then(|r| r.get("tools"))
        .and_then(Value::as_array)
    {
        for t in tools {
            if let Some(name) = t.get("name").and_then(Value::as_str) {
                if which(name).is_none() {
                    missing.push(name.to_string());
                }
            }
        }
    }
    if !missing.is_empty() {
        bail!("工具需求不满足，PATH 上缺: {}", missing.join(", "));
    }
    let shell = find_shell()?;

    let base = std::env::temp_dir().join(format!("mpkg-lyco-{}", &id[7..19]));
    std::fs::create_dir_all(&base).with_context(|| format!("建目录失败 {}", base.display()))?;
    let work = base.join("work").to_string_lossy().replace('\\', "/");
    std::fs::create_dir_all(&work).with_context(|| format!("建目录失败 {work}"))?;

    let replay = |steps_log: &mut Vec<Value>, verify_log: &mut Vec<Value>| -> Result<bool> {
        let mut ok = true;
        if let Some(steps) = manifest.get("steps").and_then(Value::as_array) {
            for (i, step) in steps.iter().enumerate() {
                let raw = step.get("run").and_then(Value::as_str).unwrap_or("");
                let want = step
                    .get("expect")
                    .and_then(|e| e.get("exit"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let (code, so, se, ms) =
                    run_timed(&shell, &subst(raw, &pkg, &work), Path::new(&work), timeout)?;
                let step_ok = i64::from(code) == want;
                steps_log.push(json!({
                    "n": i + 1, "cmd": raw, "exit": code, "ms": ms,
                    "stdout_tail": tail400(&so), "stderr_tail": tail400(&se),
                    "ok": step_ok,
                }));
                if !step_ok {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            if let Some(verifies) = manifest.get("verify").and_then(Value::as_array) {
                for cmd in verifies {
                    let raw = cmd.as_str().unwrap_or("");
                    let (code, so, se, ms) =
                        run_timed(&shell, &subst(raw, &pkg, &work), Path::new(&work), timeout)?;
                    verify_log.push(json!({
                        "cmd": raw, "exit": code, "ms": ms,
                        "stdout_tail": tail400(&so), "stderr_tail": tail400(&se),
                    }));
                    if code != 0 {
                        ok = false;
                        break;
                    }
                }
            }
        }
        Ok(ok)
    };

    let mut steps_log = Vec::new();
    let mut verify_log = Vec::new();
    let replayed = replay(&mut steps_log, &mut verify_log);
    let _ = std::fs::remove_dir_all(&base);

    let (ok_flag, err_msg) = match &replayed {
        Ok(true) => (true, None),
        Ok(false) => (false, None),
        Err(e) => (false, Some(format!("{e:#}"))),
    };

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(json!({
        "mpkg_id": id,
        "name": manifest.get("name"),
        "files": n_files,
        "ok": ok_flag,
        "steps": steps_log,
        "verify": verify_log,
        "host": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "impl": concat!("lycore ", env!("CARGO_PKG_VERSION")),
            "shell": shell,
        },
        "replayed_at": format!("{secs}s"),
        "error": err_msg,
    }))
}

// ── pack：trace ndjson → mpkg 包目录（trace-as-commits 落地）──────────────────
//
// mpkg spec §6.5「一步 = 一条 trace = 一个 commit」的格式侧兑现：
//   - `tool(ok=true, name=shell_exec)` 事件 → `steps[].run`（可回放的确定性脊）；
//   - 试错（ok=false）与 revert 不进 steps —— 只记成功 = 成品，记下试错 = 编曲；
//     编曲注记以 `provenance`（v0.1 可选字段，见 GOLDEN.md §1）随包带走；
//   - 原始 trace.ndjson **逐字节**复制进包（files 表锁死，改一个字节就是另一个包）。
// 回放语义注记：trace 里的命令原本跑在当时的 cwd，回放时跑在 {{work}} ——
// 确定性脊保真的是「命令序列与退出码」，不是文件系统环境。

/// trace ndjson → mpkg 包目录。返回 (清单, content-id, 文件数)。
///
/// `name` 缺省 `trace-<unix秒>`；`intent` 取首条 prompt 事件文本。
pub fn pack_trace(
    trace: &Path,
    out_dir: &Path,
    name: Option<&str>,
) -> Result<(Value, String, usize)> {
    let raw = fs::read_to_string(trace).with_context(|| format!("读不到 {}", trace.display()))?;
    let events: Vec<Value> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            serde_json::from_str::<Value>(l)
                .map_err(|e| anyhow!("trace 第 {} 行不是合法 JSON: {e}", l))
        })
        .collect::<Result<_>>()?;
    if events.is_empty() {
        bail!("trace 是空的: {}", trace.display());
    }

    let mut intent = String::new();
    let mut steps: Vec<Value> = Vec::new();
    let mut prompts: Vec<(u64, String)> = Vec::new(); // (事件序号, 文本)
    for ev in &events {
        match ev.get("kind").and_then(Value::as_str).unwrap_or("") {
            "prompt" => {
                let n = ev
                    .get("i")
                    .and_then(Value::as_u64)
                    .unwrap_or(prompts.len() as u64 + 1);
                let text = ev
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if intent.is_empty() {
                    intent = text.clone();
                }
                prompts.push((n, text));
            }
            // 只收成功的 shell_exec —— 其余工具的 arg 不是 shell 命令，回放不了；
            // 失败的调用是试错，不进 steps（原始字节在包内 trace.ndjson 里，一个不少）。
            "tool" => {
                let ok = ev.get("ok").and_then(Value::as_bool).unwrap_or(false);
                let tool = ev.get("name").and_then(Value::as_str).unwrap_or("");
                if ok && tool == "shell_exec" {
                    let arg = ev.get("arg").and_then(Value::as_str).unwrap_or("");
                    if !arg.is_empty() {
                        steps.push(json!({ "run": arg }));
                    }
                }
            }
            _ => {}
        }
    }
    if steps.is_empty() {
        bail!(
            "trace 里没有可回放的 shell_exec 成功步骤（共 {} 个事件）",
            events.len()
        );
    }

    let name = match name {
        Some(n) => n.trim().to_string(),
        None => {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("trace-{secs}")
        }
    };

    let manifest = json!({
        "mpkg": FORMAT,
        "name": name,
        "version": "0.1.0",
        "intent": if intent.is_empty() { "lyco_agent trace 回放包".to_string() } else { intent },
        "steps": steps,
        // 回放 cwd 是 {{work}}（Python 原型语义），查包内文件必须显式 {{pkg}} 前缀
        "verify": [
            format!("test -f {{{{pkg}}}}/{MANIFEST}"),
            "test -f {{pkg}}/trace.ndjson".to_string()
        ],
        "tags": [ "trace" ],
        // provenance 形状由 schema $defs/provenance 封死（additionalProperties: false）：
        // 只有 generator / prompts[{n,text,step_refs?}] / parent。prompt 原文落包内
        // prompts/<n>.md，text 字段指向它（schema 描述：「指向包内 prompts/*.md 原文」）。
        "provenance": {
            "generator": "ai:lyco_agent",
            "prompts": prompts
                .iter()
                .map(|(n, _)| json!({ "n": n, "text": format!("prompts/{n}.md") }))
                .collect::<Vec<_>>(),
        },
    });
    validate(&manifest)?;

    fs::create_dir_all(out_dir).with_context(|| format!("建目录失败 {}", out_dir.display()))?;
    fs::write(
        out_dir.join(MANIFEST),
        serde_json::to_string_pretty(&manifest).expect("manifest 序列化"),
    )
    .with_context(|| format!("写 {} 失败", out_dir.display()))?;
    fs::copy(trace, out_dir.join("trace.ndjson")).with_context(|| "复制 trace.ndjson 进包失败")?;
    for (n, text) in &prompts {
        let p = out_dir.join("prompts").join(format!("{n}.md"));
        fs::create_dir_all(p.parent().expect("prompts 有父目录"))?;
        fs::write(p, text)?;
    }

    package_id_of_dir(out_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── golden 向量（契约源 lystack proto/mpkg/golden/cases.json；勿手改，改格式先改契约再跑 gen_golden.py） ──

    const CASE1_CANON: &str = r#"{"manifest":{"intent":"最小合法包：仅 manifest，无 files 键","mpkg":"0.1","name":"hello-mpkg","steps":[{"run":"echo hello-mpkg"}],"verify":["true"],"version":"0.1.0"}}"#;
    const CASE1_ID: &str =
        "sha256:830d8d92aa3a7ca9655468c5fc034e10902f9aad225825fdb7e88824a7eeda58";
    const CASE1_BLOB: &str = "830d8d92aa3a7ca9655468c5fc034e10902f9aad225825fdb7e88824a7eeda58";

    const CASE2_CANON: &str = r#"{"files":{"README.md":"fa92edd9d1241161b07fce3404e6c8cceb23dd8790521e274923db0afdc14f14","artifacts/lib/util.py":"b1b3395bd5ce1ca48ec48abae5c25b1007bd4b0762717b811742579f93fe1a83","artifacts/main.py":"e1f295db15fc563c98997b625f807baf10d885df64a8ab5a4272cde6401afbd2"},"manifest":{"intent":"含 files 的多文件包：嵌套路径哈希引用","mpkg":"0.1","name":"multi-file-demo","steps":[{"expect":{"exit":0},"run":"echo multi"}],"verify":["test -f artifacts/main.py","test -f artifacts/lib/util.py"],"version":"0.2.0"}}"#;
    const CASE2_ID: &str =
        "sha256:f501b4282a8dcb3d44b3a1978ac0643f280fa5926e131e3f087204e7355febd7";

    const CASE3_CANON: &str = r#"{"files":{"artifacts/out.txt":"084c799cd551dd1d8d5c5f9a5d593b2e931f5e36122ee5c793c1d08a19839cc0"},"manifest":{"author":"agent:lilyco","intent":"步骤齐全包：expect.exit / 多 verify / requirements / 溯源可选字段","license":"MIT","mpkg":"0.1","name":"full-replay-suite","requirements":{"tools":[{"min_version":"3.10","name":"python3"}]},"steps":[{"run":"echo step-1"},{"expect":{"exit":0},"run":"python3 -c 'print(6*7)'"},{"expect":{"exit":0},"run":"test -d {{work}}"}],"tags":["demo","replay"],"verify":["test -f artifacts/out.txt","grep -q 42 artifacts/out.txt"],"version":"1.0.0"}}"#;
    const CASE3_ID: &str =
        "sha256:7bc60dd2e45e99cdd8b095f36b28234899495743001001d2c1169963bcf55086";

    /// 与 cases.json 的 pack 字段逐字段一致（canon 会重排键序，输入键序无所谓）。
    fn case1_pack() -> Value {
        json!({
            "manifest": {
                "mpkg": "0.1", "name": "hello-mpkg", "version": "0.1.0",
                "intent": "最小合法包：仅 manifest，无 files 键",
                "steps": [ { "run": "echo hello-mpkg" } ],
                "verify": [ "true" ]
            }
        })
    }
    fn case2_pack() -> Value {
        json!({
            "manifest": {
                "mpkg": "0.1", "name": "multi-file-demo", "version": "0.2.0",
                "intent": "含 files 的多文件包：嵌套路径哈希引用",
                "steps": [ { "run": "echo multi", "expect": { "exit": 0 } } ],
                "verify": [ "test -f artifacts/main.py", "test -f artifacts/lib/util.py" ]
            },
            "files": {
                "README.md": "fa92edd9d1241161b07fce3404e6c8cceb23dd8790521e274923db0afdc14f14",
                "artifacts/lib/util.py": "b1b3395bd5ce1ca48ec48abae5c25b1007bd4b0762717b811742579f93fe1a83",
                "artifacts/main.py": "e1f295db15fc563c98997b625f807baf10d885df64a8ab5a4272cde6401afbd2"
            }
        })
    }
    fn case3_pack() -> Value {
        json!({
            "manifest": {
                "mpkg": "0.1", "name": "full-replay-suite", "version": "1.0.0",
                "intent": "步骤齐全包：expect.exit / 多 verify / requirements / 溯源可选字段",
                "author": "agent:lilyco",
                "requirements": { "tools": [ { "name": "python3", "min_version": "3.10" } ] },
                "steps": [
                    { "run": "echo step-1" },
                    { "run": "python3 -c 'print(6*7)'", "expect": { "exit": 0 } },
                    { "run": "test -d {{work}}", "expect": { "exit": 0 } }
                ],
                "tags": [ "demo", "replay" ],
                "verify": [ "test -f artifacts/out.txt", "grep -q 42 artifacts/out.txt" ],
                "license": "MIT"
            },
            "files": {
                "artifacts/out.txt": "084c799cd551dd1d8d5c5f9a5d593b2e931f5e36122ee5c793c1d08a19839cc0"
            }
        })
    }

    /// NIST FIPS 180 向量：算法错了这里先红（自实现 sha256 的正确性锚点）。
    #[test]
    fn sha256_nist_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// GOLDEN.md §6 验收：canon_json 逐字节一致（三例）。
    #[test]
    fn golden_canon_byte_exact() {
        assert_eq!(canon(&case1_pack()), CASE1_CANON);
        assert_eq!(canon(&case2_pack()), CASE2_CANON);
        assert_eq!(canon(&case3_pack()), CASE3_CANON);
    }

    /// GOLDEN.md §6 验收：content-id 逐字节一致（三例）。
    #[test]
    fn golden_content_id_byte_exact() {
        assert_eq!(content_id(&case1_pack()), CASE1_ID);
        assert_eq!(content_id(&case2_pack()), CASE2_ID);
        assert_eq!(content_id(&case3_pack()), CASE3_ID);
    }

    /// GOLDEN.md §6 验收：golden 三例的 manifest 必须过本实现的 validate。
    #[test]
    fn golden_manifests_pass_validate() {
        for (i, pack) in [case1_pack(), case2_pack(), case3_pack()]
            .iter()
            .enumerate()
        {
            let m = pack.get("manifest").expect("golden 包必有 manifest");
            assert!(
                validate(m).is_ok(),
                "golden 用例 {i} 的 manifest 应过 validate"
            );
        }
    }

    /// canon 幂等：对 canon 输出再解析再 canon，字节不变。
    #[test]
    fn canon_is_idempotent() {
        for pack in [case1_pack(), case2_pack(), case3_pack()] {
            let once = canon(&pack);
            let reparsed: Value = serde_json::from_str(&once).expect("canon 输出必须是合法 JSON");
            assert_eq!(canon(&reparsed), once);
        }
    }

    /// GOLDEN.md §3 两层哈希：非 canon 字节落盘时 blob 变而 content-id 不变。
    /// 输入是 canon 形态时两者十六进制恰好相同（golden 即此形态）。
    #[test]
    fn two_layer_hash_diverge_on_non_canon_bytes() {
        let pretty = serde_json::to_string_pretty(&case1_pack()).expect("pretty 序列化");
        let blob = sha256_hex(pretty.as_bytes());
        assert_ne!(blob, CASE1_BLOB, "pretty 字节变了，blob 必须变");
        assert_eq!(
            content_id(&case1_pack()),
            CASE1_ID,
            "content-id 对空白不敏感"
        );
    }

    /// 校验器咬人：缺字段 / 版本不符 / 非 kebab-case / 空 steps / verify 元素非字符串。
    #[test]
    fn validate_rejects_garbage() {
        let base = case1_pack();
        let m = base["manifest"].clone();

        let mut no_verify = m.clone();
        no_verify.as_object_mut().unwrap().remove("verify");
        assert!(validate(&no_verify).is_err());

        let mut bad_ver = m.clone();
        bad_ver["mpkg"] = json!("9.9");
        assert!(validate(&bad_ver).is_err());

        let mut bad_name = m.clone();
        bad_name["name"] = json!("Hello_World");
        assert!(validate(&bad_name).is_err());

        let mut empty_steps = m.clone();
        empty_steps["steps"] = json!([]);
        assert!(validate(&empty_steps).is_err());

        let mut bad_verify = m.clone();
        bad_verify["verify"] = json!([42]);
        assert!(validate(&bad_verify).is_err());

        assert!(validate(&m).is_ok(), "基准包必须过");
    }

    // ── verify（回放）─────────────────────────────────────────────────────

    /// 建一个临时包目录（mpkg.json + 可选附加文件），返回目录路径。
    /// 测试结束由调用方清理（返回 base 以便 remove_dir_all）。
    fn make_pkg(name: &str, manifest: Value, files: &[(&str, &str)]) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("mpkg-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("建测试包目录");
        std::fs::write(
            base.join(MANIFEST),
            serde_json::to_string_pretty(&manifest).expect("manifest 序列化"),
        )
        .expect("写 manifest");
        for (rel, content) in files {
            let p = base.join(rel);
            std::fs::create_dir_all(p.parent().expect("有父目录")).expect("建子目录");
            std::fs::write(p, content).expect("写附加文件");
        }
        base
    }

    fn test_manifest(steps: Value, verify: Value) -> Value {
        json!({
            "mpkg": "0.1",
            "name": "verify-test-pkg",
            "version": "0.1.0",
            "intent": "verify 集成测试包",
            "steps": steps,
            "verify": verify
        })
    }

    #[test]
    fn verify_dir_happy_path() {
        let base = make_pkg(
            "happy",
            test_manifest(
                json!([{ "run": "echo hi from step" }]),
                json!(["test -f {{pkg}}/mpkg.json"]),
            ),
            &[("artifacts/out.txt", "42\n")],
        );
        let att = verify_dir(&base).expect("回放应成功执行");
        assert_eq!(att["ok"], true, "attestation: {att}");
        assert_eq!(att["steps"][0]["exit"], 0);
        assert_eq!(att["steps"][0]["ok"], true);
        assert!(att["steps"][0]["stdout_tail"]
            .as_str()
            .unwrap()
            .contains("hi from step"));
        assert_eq!(att["verify"][0]["exit"], 0);
        assert!(att["mpkg_id"].as_str().unwrap().starts_with("sha256:"));
        // content-id 与 package_id_of_dir 复算一致（同一包两次定身同 id）
        let (_, id2, _) = package_id_of_dir(&base).unwrap();
        assert_eq!(att["mpkg_id"].as_str().unwrap(), id2);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verify_dir_step_fail_stops_and_reports() {
        let base = make_pkg(
            "stepfail",
            test_manifest(json!([{ "run": "exit 7" }]), json!(["true"])),
            &[],
        );
        let att = verify_dir(&base).expect("回放失败也是正常结论，不应 Err");
        assert_eq!(att["ok"], false);
        assert_eq!(att["steps"][0]["exit"], 7);
        assert_eq!(att["steps"][0]["ok"], false);
        assert!(
            att["verify"].as_array().unwrap().is_empty(),
            "step 挂了就不再跑 verify"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verify_dir_expect_exit_tolerates_nonzero() {
        let base = make_pkg(
            "expect3",
            test_manifest(
                json!([{ "run": "exit 3", "expect": { "exit": 3 } }]),
                json!(["true"]),
            ),
            &[],
        );
        let att = verify_dir(&base).expect("回放应成功执行");
        assert_eq!(att["ok"], true, "attestation: {att}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verify_dir_verify_fail_is_ok_false() {
        let base = make_pkg(
            "vfail",
            test_manifest(
                json!([{ "run": "true" }]),
                json!(["test -f {{pkg}}/不存在的文件"]),
            ),
            &[],
        );
        let att = verify_dir(&base).expect("回放失败也是正常结论");
        assert_eq!(att["ok"], false);
        assert_eq!(att["verify"][0]["exit"], 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verify_dir_timeout_kills_runaway_step() {
        let base = make_pkg(
            "slow",
            test_manifest(json!([{ "run": "sleep 30" }]), json!(["true"])),
            &[],
        );
        // 超时是回放结论不是程序错误 → 进 attestation 的 error 字段，ok=false
        let att = verify_dir_with(&base, std::time::Duration::from_secs(1))
            .expect("超时也应有 attestation");
        assert_eq!(att["ok"], false);
        assert!(
            att["error"].as_str().unwrap().contains("超时"),
            "应报超时: {}",
            att["error"]
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verify_dir_missing_tool_refuses() {
        let mut m = test_manifest(json!([{ "run": "true" }]), json!(["true"]));
        m["requirements"] = json!({ "tools": [ { "name": "definitely-no-such-tool-xyz-42" } ] });
        let base = make_pkg("notool", m, &[]);
        let err = verify_dir(&base).unwrap_err();
        assert!(
            err.to_string().contains("definitely-no-such-tool-xyz-42"),
            "报错应点名缺的工具: {err}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // ── pack_trace（trace → 包，trace-as-commits）──────────────────────────

    /// trace→打包→定身→回放 全闭环：Tracer 造 ndjson，pack_trace 出包，
    /// manifest 语义逐项断言，最后 verify_dir 真回放里面的 steps。
    #[test]
    fn pack_trace_full_loop() {
        use crate::trace::Tracer;

        let tmp = std::env::temp_dir().join(format!("mpkg-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let trace_path = tmp.join("s.trace.ndjson");
        let mut tr = Tracer::new(&trace_path);
        tr.prompt("帮我写贪吃蛇");
        tr.tool("shell_exec", "echo step-one", true);
        tr.tool("shell_exec", "cargo build 不存在", false); // 试错：不进 steps
        tr.tool("read_file", "src/main.rs", true); // 非 shell 工具：不进 steps
        tr.revert("编译失败: 缺 use");
        tr.tool("shell_exec", "echo step-two", true);
        tr.final_answer("完成");

        let out = tmp.join("pkg");
        let (manifest, id, n_files) =
            pack_trace(&trace_path, &out, Some("trace-snake")).expect("打包应成功");

        // manifest 语义
        assert_eq!(manifest["name"], "trace-snake");
        assert_eq!(manifest["intent"], "帮我写贪吃蛇");
        let steps = manifest["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 2, "只收 ok=true 的 shell_exec");
        assert_eq!(steps[0]["run"], "echo step-one");
        assert_eq!(steps[1]["run"], "echo step-two");
        // provenance 是 schema 封闭形状
        let prov = &manifest["provenance"];
        assert_eq!(prov["generator"], "ai:lyco_agent");
        assert_eq!(prov["prompts"][0]["text"], "prompts/1.md");
        // 包内容物
        assert!(out.join("trace.ndjson").is_file(), "原始 trace 逐字节进包");
        assert!(out.join("prompts/1.md").is_file());
        assert!(n_files >= 3, "mpkg.json + trace.ndjson + prompts/*.md");
        assert!(id.starts_with("sha256:"));

        // 打出的包必须能回放：steps 是真命令，verify 查包内文件
        let att = verify_dir(&out).expect("回放应成功执行");
        assert_eq!(att["ok"], true, "attestation: {att}");
        assert_eq!(att["mpkg_id"], id.as_str(), "定身两次同 id");
        assert_eq!(att["steps"].as_array().unwrap().len(), 2);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn pack_trace_rejects_unreplayable() {
        use crate::trace::Tracer;

        let tmp = std::env::temp_dir().join(format!("mpkg-pack2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let trace_path = tmp.join("s.trace.ndjson");
        let mut tr = Tracer::new(&trace_path);
        tr.prompt("只问了不干");
        tr.final_answer("答完了");

        let out = tmp.join("pkg");
        let err = pack_trace(&trace_path, &out, None).unwrap_err();
        assert!(
            err.to_string().contains("shell_exec"),
            "应说明缺可回放步骤: {err}"
        );

        // 空文件同样拒
        let empty = tmp.join("empty.ndjson");
        std::fs::write(&empty, "").unwrap();
        assert!(pack_trace(&empty, &out, None).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn pack_trace_default_name_is_kebab() {
        use crate::trace::Tracer;

        let tmp = std::env::temp_dir().join(format!("mpkg-pack3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let trace_path = tmp.join("s.trace.ndjson");
        let mut tr = Tracer::new(&trace_path);
        tr.prompt("写贪吃蛇");
        tr.tool("shell_exec", "echo hi", true);

        let out = tmp.join("pkg");
        let (manifest, _, _) = pack_trace(&trace_path, &out, None).expect("打包应成功");
        let name = manifest["name"].as_str().unwrap();
        assert!(
            name.starts_with("trace-")
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "缺省名须过 validate 的 kebab-case: {name}"
        );
        assert!(validate(&manifest).is_ok());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
