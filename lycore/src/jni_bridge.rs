//! JNI 桥 —— Android APK 直接调用 lycore 的**确定性内核**。
//!
//! ## 为什么是 JNI 而不是"在 Java 里重写一遍"
//!
//! `help_parse` / `cli_zh` / `choices` / `t1gate` 是 lycore 的**单一真源**。
//! 在 Java 侧重写等于制造第二份实现，两边必然漂移 —— 而漂移正是 v17 的
//! F1「掉前缀」、v19b「方法写对了但调用点没换」这一类事故的共同根因。
//!
//! ## 暴露给 Java 的三个入口
//!
//! | 函数 | 用途 | 「关灯」链路里的位置 |
//! |---|---|---|
//! | `parseHelp` | `--help` 原文 → 动作表 JSON | 拿到含 `hw led blue off` 的候选集 |
//! | `manual` | `--help` 原文 → 中文说明书 | 给人看 / 做训练数据 |
//! | `gate` | 命令 → 风险与缺参判定 | 执行前的最后一道闸 |
//!
//! ⚠️ 全部**不含模型**：这层是纯确定性代码，离线可用、零 GPU。
//! 模型（Qwen）将来只接在"人话 → 选编号"那一环，不进这里。

use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;

/// JString → String（取不到就 None，绝不 panic 进 JVM）
fn to_rust_string<'local>(env: &mut JNIEnv<'local>, s: &JString<'local>) -> Option<String> {
    env.get_string(s).ok().map(|j| j.into())
}

/// String → jstring（失败返回 null，让 Java 侧自己判空）
fn to_jstring<'local>(env: &mut JNIEnv<'local>, s: String) -> jstring {
    match env.new_string(s) {
        Ok(j) => j.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// `lyco.Lycore.parseHelp(cli, helpText) -> JSON`
///
/// 把 `cli --help` 的原文解析成动作表。**确定性，与 PC 端完全同一份代码**。
///
/// 返回字段见 [`crate::help_parse::HelpAction`]：`full_cmd` / `desc` /
/// `readonly` / `example`。解析不出任何动作时返回 `[]`（**绝不返回垃圾**）。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_lyco_Lycore_parseHelp<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    cli: JString<'local>,
    help: JString<'local>,
) -> jstring {
    let Some(c) = to_rust_string(&mut env, &cli) else {
        return std::ptr::null_mut();
    };
    let Some(h) = to_rust_string(&mut env, &help) else {
        return std::ptr::null_mut();
    };
    let acts = crate::help_parse::parse_help(&c, &h);
    let json = serde_json::to_string(&acts).unwrap_or_else(|_| "[]".to_string());
    to_jstring(&mut env, json)
}

/// `lyco.Lycore.manual(cli, helpText) -> Markdown 中文说明书`
///
/// 解析 → 本地词典翻译（**零网络零模型**）→ 渲染。
/// 说明书里混排的英文就是词典没翻的部分，`coverage` 字段量化了翻译质量。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_lyco_Lycore_manual<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    cli: JString<'local>,
    help: JString<'local>,
) -> jstring {
    let Some(c) = to_rust_string(&mut env, &cli) else {
        return std::ptr::null_mut();
    };
    let Some(h) = to_rust_string(&mut env, &help) else {
        return std::ptr::null_mut();
    };
    let acts = crate::help_parse::parse_help(&c, &h);
    if acts.is_empty() {
        // 与 `lycore manual` 同纪律：解析不出东西就不产出说明书，不编造
        return to_jstring(&mut env, String::new());
    }
    let zh: Vec<crate::cli_zh::ZhAction> =
        acts.iter().map(crate::cli_zh::translate_action).collect();
    let md = crate::cli_zh::render_manual(&c, &zh, h.len());
    to_jstring(&mut env, md)
}

/// `lyco.Lycore.gate(cmd) -> JSON {risk, decision, reason, param}`
///
/// 执行前的最后一道闸：风险分级（`read`/`write`/`danger`）+ 缺参检查。
/// 与 `lycore t1 check` 同一套规则 —— **这是"关灯"不被误放行成"删库"的保障**。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_lyco_Lycore_gate<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    cmd: JString<'local>,
) -> jstring {
    let Some(c) = to_rust_string(&mut env, &cmd) else {
        return std::ptr::null_mut();
    };
    let v = crate::t1gate::classify(&c);
    let p = crate::paramcheck::check(&c);
    let decision = match v.risk {
        crate::t1gate::Risk::Danger => "block",
        _ if p.status.is_needs_param() => "needs_param",
        crate::t1gate::Risk::Write => "confirm",
        crate::t1gate::Risk::Read => "run",
    };
    let json = serde_json::json!({
        "command": v.command,
        "risk": v.risk.as_str(),
        "decision": decision,
        "matched": v.matched,
        "reason": v.reason,
        "param": {
            "status": p.status.as_str(),
            "missing": p.missing,
            "reason": p.reason,
        },
    });
    to_jstring(&mut env, json.to_string())
}

/// `lyco.Lycore.version() -> &str`
///
/// 冒烟用：APK 起来先调一次，确认 `.so` 真的加载了、JNI 名字没对错。
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_lyco_Lycore_version<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    to_jstring(&mut env, env!("CARGO_PKG_VERSION").to_string())
}
