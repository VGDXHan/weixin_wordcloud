//! FFI wrapper around WeFlow's `wx_key.dll` to obtain the WeChat DB key.
//!
//! Reproduces keyService.ts's flow: InitializeHook(pid) then poll PollKeyData
//! until a 64-hex key appears. Requires the app to run with administrator
//! privileges (the DLL hooks the WeChat process).

use crate::error::{AppError, AppResult};
use libloading::{Library, Symbol};
use std::ffi::{c_char, c_int, CStr};
use std::path::Path;
use std::time::{Duration, Instant};

type InitHookFn = unsafe extern "C" fn(u32) -> bool;
type PollKeyFn = unsafe extern "C" fn(*mut c_char, c_int) -> bool;
type StatusFn = unsafe extern "C" fn(*mut c_char, c_int, *mut c_int) -> bool;
type CleanupFn = unsafe extern "C" fn() -> bool;
type LastErrFn = unsafe extern "C" fn() -> *const c_char;

/// Pick the main WeChat process: several `Weixin.exe` processes run (helpers
/// use ~0-1 MB), the real UI/DB process has by far the largest working set.
/// This mirrors WeFlow targeting the process that owns the "微信" window.
fn find_wechat_pid() -> Option<u32> {
    let mut best: Option<(u32, u64)> = None; // (pid, mem KB)
    for name in ["Weixin.exe", "WeChat.exe"] {
        let Ok(out) = std::process::Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {name}"), "/FO", "CSV", "/NH"])
            .output()
        else {
            continue;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("INFO:") {
                continue;
            }
            // "Weixin.exe","1234","Console","1","62,000 K"
            let parts: Vec<&str> = line.split("\",\"").map(|s| s.trim_matches('"')).collect();
            if parts.len() < 2 || !parts[0].eq_ignore_ascii_case(name) {
                continue;
            }
            let Ok(pid) = parts[1].trim().parse::<u32>() else {
                continue;
            };
            let mem: u64 = parts
                .get(4)
                .map(|s| {
                    s.chars()
                        .filter(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse()
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            if best.as_ref().map(|(_, m)| mem > *m).unwrap_or(true) {
                best = Some((pid, mem));
            }
        }
    }
    best.map(|(pid, _)| pid)
}

/// Check whether a given PID is still running.
fn pid_alive(pid: u32) -> bool {
    let Ok(out) = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
    else {
        return true; // assume alive if we can't tell
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().any(|l| l.contains(&format!("\"{pid}\"")))
}

/// Extract the 64-hex DB key using wx_key.dll.
///
/// The DLL captures the key when WeChat *derives* it (i.e. at login / first DB
/// open). If WeChat is already logged in, that won't fire again — so we mirror
/// WeFlow: install the hook, and if the hooked process exits (user restarts
/// WeChat) we re-hook the new process and catch the key during its login.
/// `timeout` bounds the whole flow.
pub fn get_db_key(wx_key_dll: &Path, timeout: Duration) -> AppResult<String> {
    let lib = unsafe { Library::new(wx_key_dll) }
        .map_err(|e| AppError::DllLoad(format!("wx_key.dll: {e}")))?;

    unsafe {
        let init_hook: Symbol<InitHookFn> = lib
            .get(b"InitializeHook\0")
            .map_err(|e| AppError::DllLoad(format!("InitializeHook: {e}")))?;
        let poll_key: Symbol<PollKeyFn> = lib
            .get(b"PollKeyData\0")
            .map_err(|e| AppError::DllLoad(format!("PollKeyData: {e}")))?;
        let cleanup: Symbol<CleanupFn> = lib
            .get(b"CleanupHook\0")
            .map_err(|e| AppError::DllLoad(format!("CleanupHook: {e}")))?;
        let get_status: Option<Symbol<StatusFn>> = lib.get(b"GetStatusMessage\0").ok();
        let last_err: Option<Symbol<LastErrFn>> = lib.get(b"GetLastErrorMsg\0").ok();

        let read_last_err = || -> String {
            if let Some(f) = &last_err {
                let p = f();
                if !p.is_null() {
                    return CStr::from_ptr(p).to_string_lossy().into_owned();
                }
            }
            String::new()
        };

        let deadline = Instant::now() + timeout;
        let mut last_status = String::new();
        let mut buf = vec![0i8; 128];
        let mut sbuf = vec![0i8; 512];

        // Outer loop: (re)acquire a WeChat pid and hook it.
        loop {
            if Instant::now() >= deadline {
                break;
            }
            let Some(pid) = find_wechat_pid() else {
                eprintln!("[wx_key] 等待微信进程…");
                std::thread::sleep(Duration::from_millis(600));
                continue;
            };

            if !init_hook(pid) {
                let msg = read_last_err();
                cleanup();
                if !pid_alive(pid) && Instant::now() < deadline {
                    continue; // process died mid-hook; retry with new one
                }
                return Err(AppError::KeyFailed(if msg.is_empty() {
                    "初始化 Hook 失败，请以管理员身份运行，并关闭杀毒软件拦截".into()
                } else {
                    msg
                }));
            }
            eprintln!("[wx_key] Hook 已安装于 pid={pid}，轮询密钥中…");

            let mut process_ended = false;
            let mut next_alive_check = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                if poll_key(buf.as_mut_ptr() as *mut c_char, buf.len() as c_int) {
                    let key = CStr::from_ptr(buf.as_ptr() as *const c_char)
                        .to_string_lossy()
                        .trim()
                        .to_string();
                    if key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit()) {
                        cleanup();
                        eprintln!("[wx_key] 密钥获取成功");
                        return Ok(key);
                    }
                }

                // Drain DLL status messages (progress / login hints).
                if let Some(f) = &get_status {
                    let mut level: c_int = 0;
                    for _ in 0..8 {
                        if !f(sbuf.as_mut_ptr() as *mut c_char, sbuf.len() as c_int, &mut level) {
                            break;
                        }
                        let msg = CStr::from_ptr(sbuf.as_ptr() as *const c_char)
                            .to_string_lossy()
                            .trim()
                            .to_string();
                        if !msg.is_empty() {
                            eprintln!("[wx_key] status(level={level}): {msg}");
                            last_status = msg;
                        }
                    }
                }

                if Instant::now() >= next_alive_check {
                    next_alive_check = Instant::now() + Duration::from_secs(1);
                    if !pid_alive(pid) {
                        eprintln!("[wx_key] 微信进程已退出，等待重新登录后重挂 Hook…");
                        process_ended = true;
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(120));
            }

            cleanup();
            if process_ended {
                continue; // re-hook the freshly started WeChat
            }
            break; // deadline hit
        }

        let err = read_last_err();
        let detail = if !err.is_empty() {
            err
        } else if !last_status.is_empty() {
            last_status
        } else {
            "无 DLL 状态输出".into()
        };
        Err(AppError::KeyFailed(format!(
            "获取密钥超时（DLL 状态：{detail}）。请在软件显示『正在初始化』时，彻底退出并重新登录微信（Hook 在登录瞬间抓取密钥）"
        )))
    }
}
