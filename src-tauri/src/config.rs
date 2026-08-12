//! Locate the only native library we still reuse from WeFlow: `wx_key.dll`
//! (extracts the WeChat DB key). Everything else — decryption and reading — is
//! implemented ourselves in Rust.

use crate::error::{AppError, AppResult};
use std::path::PathBuf;

fn candidate_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();

    if let Some(p) = std::env::var_os("WX_KEY_DLL") {
        v.push(PathBuf::from(p));
    }

    // Bundled next to our own executable / in src-tauri/resources (dev).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            v.push(d.join("resources").join("wx_key.dll"));
            v.push(d.join("wx_key.dll"));
            if let Some(p) = d.parent().and_then(|p| p.parent()) {
                v.push(p.join("resources").join("wx_key.dll"));
            }
        }
    }
    v.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("wx_key.dll"));

    // Fall back to an installed WeFlow.
    let mut roots = Vec::new();
    if let Some(d) = std::env::var_os("WEFLOW_DIR") {
        roots.push(PathBuf::from(d));
    }
    roots.push(PathBuf::from(r"D:\Soft\weflow"));
    for env in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(base) = std::env::var_os(env) {
            let b = PathBuf::from(base);
            roots.push(b.join("Programs").join("weflow"));
            roots.push(b.join("Programs").join("WeFlow"));
        }
    }
    for r in roots {
        v.push(
            r.join("resources").join("resources").join("key").join("win32").join("x64").join("wx_key.dll"),
        );
    }

    v
}

pub fn find_wx_key_dll() -> AppResult<PathBuf> {
    let cands = candidate_paths();
    for p in &cands {
        if p.is_file() {
            return Ok(p.clone());
        }
    }
    Err(AppError::DllNotFound(
        "未找到 wx_key.dll（应位于 src-tauri/resources 或已安装的 WeFlow 中）".into(),
    ))
}
