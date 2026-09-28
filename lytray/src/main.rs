//! lytray —— lyco 托盘壳（仿 Catime 的轻量路线：单文件、无控制台、双击即用）。
//!
//! 小白交互面：托盘图标右键 → 菜单点一下 = 干活。全程不出现命令行。
//! 干活本体是 lycore 库（同仓 path 依赖），GUI 只是壳 —— 逻辑不重复实现。
//!
//! 包布局约定（与分发 zip 一致）：
//! ```text
//! lyco/
//! ├── lytray.exe   ← 本体
//! ├── lycore.exe   ← CLI（菜单「打开终端」时把它加进 PATH）
//! └── packs/       ← mpkg 记忆包（lab-python-env 等）
//! ```
//! `--smoke` 参数：起托盘 500ms 后自退（CI 自动冒烟用，不需要人点菜单）。

#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetCursorPos, GetMessageW, LoadCursorW, LoadIconW, MessageBoxW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetForegroundWindow, TrackPopupMenu, TranslateMessage, HMENU, IDC_ARROW,
    IDI_APPLICATION, MB_ICONINFORMATION, MB_OK, MF_SEPARATOR, MF_STRING, MSG, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WM_APP, WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
    WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};

const WM_TRAYICON: u32 = WM_APP + 1;
const ID_MENU_BOOTSTRAP: u32 = 101; // 一键机房自举
const ID_MENU_OPEN_PACKS: u32 = 102; // 打开包目录
const ID_MENU_OPEN_TERM: u32 = 103; // 打开终端
const ID_MENU_ABOUT: u32 = 104;
const ID_MENU_QUIT: u32 = 201;

/// 自举任务防重入（单实例托盘程序，全局静态即全局真相）
static RUNNING: AtomicBool = AtomicBool::new(false);

/// exe 所在目录（lytray.exe / lycore.exe / packs/ 都在这）
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn packs_dir() -> PathBuf {
    exe_dir().join("packs")
}

/// 宽字符胶水：&str → 以 NUL 结尾的 UTF-16
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn msg_box(hwnd: HWND, title: &str, text: &str) {
    unsafe {
        let t = wide(title);
        let b = wide(text);
        let _ = MessageBoxW(
            hwnd,
            PCWSTR(b.as_ptr()),
            PCWSTR(t.as_ptr()),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

/// 菜单一：一键机房自举。后台线程跑 verify（不卡消息循环），干完弹结果框。
/// RUNNING 原子位防重入（连点两次菜单不会叠两个任务）。
fn action_bootstrap(hwnd: HWND) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let pack = packs_dir().join("lab-python-env");
    if !pack.join("mpkg.json").is_file() {
        RUNNING.store(false, Ordering::SeqCst);
        msg_box(
            hwnd,
            "lyco",
            "找不到记忆包 packs/lab-python-env。\n请确认 lytray.exe 与 packs 目录在一起（解压后别拆散）。",
        );
        return;
    }
    std::thread::spawn(move || {
        let att = lycore::mpkg::verify_dir(&pack);
        RUNNING.store(false, Ordering::SeqCst);
        let text = match att {
            Ok(att) => {
                let mut lines = Vec::new();
                if att["ok"] == true {
                    lines.push("✅ 机房自举完成 —— Python 环境就绪".to_string());
                } else {
                    lines.push("❌ 自举失败（详情见下）".to_string());
                }
                if let Some(err) = att["error"].as_str() {
                    lines.push(format!("错误: {err}"));
                }
                if let Some(steps) = att["steps"].as_array() {
                    for s in steps {
                        lines.push(format!(
                            "  step {}: {} {} ({} ms)",
                            s["n"],
                            if s["ok"] == true { "✓" } else { "✗" },
                            s["cmd"].as_str().unwrap_or(""),
                            s["ms"]
                        ));
                    }
                }
                lines.push(String::new());
                lines.push(format!(
                    "包: {} ({})",
                    att["name"].as_str().unwrap_or("?"),
                    att["mpkg_id"].as_str().unwrap_or("?")
                ));
                lines.join("\n")
            }
            Err(e) => format!("❌ 回放无法开始: {e:#}"),
        };
        msg_box(HWND::default(), "lyco — 机房自举", &text);
    });
}

/// 菜单二：资源管理器打开包目录（没有就先建，保证能看到东西）。
fn action_open_packs() {
    let dir = packs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
}

/// 菜单三：开一个 lycore 就绪的终端（PATH 前置 exe 目录，小白直接敲命令）。
fn action_open_term() {
    let dir = exe_dir();
    let _ = std::process::Command::new("cmd")
        .args([
            "/K",
            "title lyco && echo lycore 已就绪，输入 lycore 查看命令。",
        ])
        .current_dir(dir.parent().unwrap_or(&dir))
        .env(
            "PATH",
            format!(
                "{};{}",
                dir.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .spawn();
}

unsafe fn add_menu(menu: HMENU, id: u32, text: &str) {
    let w = wide(text);
    let _ = AppendMenuW(menu, MF_STRING, id as usize, PCWSTR(w.as_ptr()));
}

/// 弹托盘菜单，返回被点中的命令 id（TPM_RETURNCMD：同步返回，不走 WM_COMMAND）。
unsafe fn show_menu(hwnd: HWND) -> u32 {
    let menu: HMENU = CreatePopupMenu().expect("CreatePopupMenu failed");
    add_menu(menu, ID_MENU_BOOTSTRAP, "一键机房自举（Python 环境）");
    add_menu(menu, ID_MENU_OPEN_PACKS, "打开包目录");
    add_menu(menu, ID_MENU_OPEN_TERM, "打开终端（lycore 就绪）");
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    add_menu(menu, ID_MENU_ABOUT, "关于 lyco");
    add_menu(menu, ID_MENU_QUIT, "退出");
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    // 托盘菜单三件套：前台化 → TrackPopupMenu → WM_NULL（否则点外面菜单不消失）
    let _ = SetForegroundWindow(hwnd);
    let picked = TrackPopupMenu(
        menu,
        TPM_RIGHTBUTTON | TPM_RETURNCMD,
        pt.x,
        pt.y,
        0,
        hwnd,
        None,
    );
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
    picked.0 as u32
}

const ABOUT_TEXT: &str = "lyco —— 本地 Agent（小白版）\n模型与记忆都在你自己的机器上，不需要 API token。\nmpkg 记忆包：可回放、可验证、内容寻址。\n\nhttps://lain42.top/lyco-dl/";

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            if lp.0 as u32 == WM_RBUTTONUP || lp.0 as u32 == WM_LBUTTONUP {
                match show_menu(hwnd) {
                    ID_MENU_BOOTSTRAP => action_bootstrap(hwnd),
                    ID_MENU_OPEN_PACKS => action_open_packs(),
                    ID_MENU_OPEN_TERM => action_open_term(),
                    ID_MENU_ABOUT => msg_box(hwnd, "关于 lyco", ABOUT_TEXT),
                    ID_MENU_QUIT => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let nid = NOTIFYICONDATAW {
                hWnd: hwnd,
                uID: 1,
                ..Default::default()
            };
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn main() {
    let smoke = std::env::args().any(|a| a == "--smoke");

    unsafe {
        let hmodule = GetModuleHandleW(None).expect("GetModuleHandleW failed");
        let hinstance = HINSTANCE(hmodule.0); // 0.58 里两者是不同的新类型，位宽相同

        let class_name = wide("lyco_tray");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            hIcon: LoadIconW(hinstance, IDI_APPLICATION).unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let atom = RegisterClassExW(&wc);
        assert!(atom != 0, "RegisterClassExW failed");

        let title = wide("lyco");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            0,
            0,
            None,
            None,
            hinstance,
            None,
        )
        .expect("CreateWindowExW failed");

        // 托盘图标（系统默认图标 —— v0 不带美术资源）
        let mut nid = NOTIFYICONDATAW {
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAYICON,
            hIcon: LoadIconW(hinstance, IDI_APPLICATION).unwrap_or_default(),
            ..Default::default()
        };
        let tip = wide("lyco — 本地 Agent");
        let n = tip.len().min(nid.szTip.len());
        nid.szTip[..n].copy_from_slice(&tip[..n]);
        Shell_NotifyIconW(NIM_ADD, &nid)
            .ok()
            .expect("Shell_NotifyIconW failed");

        if smoke {
            // CI 冒烟：托盘挂上 500ms 后干净退出
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            std::process::exit(0);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
