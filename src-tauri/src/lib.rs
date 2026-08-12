mod config;
mod dbcrypt;
mod dbread;
mod error;
mod export;
mod ffi_key;
mod locate;
mod mock;
mod model;
mod timefmt;
mod wordcloud;

use error::{AppError, AppResult};
use model::{ChatMessage, Diagnostics, ExportResult, Session, WordFreq, WxStatus};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long we wait for wx_key.dll to catch the key (covers a WeChat re-login).
const KEY_TIMEOUT: Duration = Duration::from_secs(180);

enum Backend {
    Mock,
    Real(RealCtx),
}

struct RealCtx {
    wxid: String,
    /// Account-level key captured by wx_key.dll. WeChat derives a separate
    /// cipher from this key and each database's own salt.
    raw_key: [u8; 32],
    temp: PathBuf,
    session_db_dec: PathBuf,
    contact_db_dec: Option<PathBuf>,
    /// Encrypted message DBs (decrypted lazily on first word-cloud build).
    message_dbs_enc: Vec<PathBuf>,
    message_dbs_dec: Vec<PathBuf>,
    message_errors: Vec<String>,
    messages_ready: bool,
}

impl Drop for RealCtx {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp);
    }
}

/// `Arc` so blocking work can take the state onto a worker thread instead of
/// freezing the UI thread (key extraction alone can block for minutes).
#[derive(Default)]
struct AppState {
    backend: Arc<Mutex<Option<Backend>>>,
}

type SharedBackend = Arc<Mutex<Option<Backend>>>;

/// Lock the backend without panicking: a poisoned mutex (a previous command
/// panicked while holding it) must surface as a readable error, not take the
/// whole app down.
fn lock_backend(shared: &SharedBackend) -> AppResult<std::sync::MutexGuard<'_, Option<Backend>>> {
    shared
        .lock()
        .map_err(|_| AppError::Other("内部状态已损坏，请重启应用".into()))
}

fn parse_key(hex_key: &str) -> AppResult<[u8; 32]> {
    let bytes = hex::decode(hex_key.trim())
        .map_err(|_| AppError::KeyFailed("密钥不是合法的十六进制".into()))?;
    if bytes.len() != 32 {
        return Err(AppError::KeyFailed(format!("密钥长度应为 32 字节，实际 {}", bytes.len())));
    }
    let mut k = [0u8; 32];
    k.copy_from_slice(&bytes);
    Ok(k)
}

/// Remove old plaintext copies left by crashed runs. A generous age threshold
/// prevents one running app instance from deleting another instance's files.
fn clean_stale_temp() {
    const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
    let base = std::env::temp_dir();
    let mine = format!("weixin_wordcloud_{}", std::process::id());
    let Ok(entries) = std::fs::read_dir(&base) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let stale = e
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .map(|age| age >= STALE_AFTER)
            .unwrap_or(false);
        if name.starts_with("weixin_wordcloud_") && name != mine && stale {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

fn try_init_real() -> AppResult<RealCtx> {
    clean_stale_temp();

    let account = locate::find_account()?;
    let dll = config::find_wx_key_dll()?;
    let key_hex = ffi_key::get_db_key(&dll, KEY_TIMEOUT)?;
    let raw_key = parse_key(&key_hex)?;

    let temp = std::env::temp_dir().join(format!("weixin_wordcloud_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp)?;

    let session_db_dec = decrypt_database(&account.session_db, &raw_key, &temp)?;
    let contact_db_dec = account
        .contact_db
        .as_ref()
        .and_then(|db| match decrypt_database(db, &raw_key, &temp) {
            Ok(path) => Some(path),
            Err(error) => {
                eprintln!("[dbcrypt] 联系人数据库解密失败：{error}");
                None
            }
        });

    Ok(RealCtx {
        wxid: account.wxid,
        raw_key,
        temp,
        session_db_dec,
        contact_db_dec,
        message_dbs_enc: account.message_dbs,
        message_dbs_dec: Vec::new(),
        message_errors: Vec::new(),
        messages_ready: false,
    })
}

fn decrypt_database(db: &std::path::Path, raw_key: &[u8; 32], out_dir: &std::path::Path) -> AppResult<PathBuf> {
    // Every WeChat 4.x DB has its own salt and therefore its own derived AES
    // and HMAC keys. Reusing session.db's Cipher for message DBs cannot work.
    let cipher = dbcrypt::detect(db, raw_key)?;
    dbcrypt::decrypt_to(db, &cipher, out_dir)
}

fn ensure_messages(ctx: &mut RealCtx) {
    if ctx.messages_ready {
        return;
    }
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for db in &ctx.message_dbs_enc {
        match decrypt_database(db, &ctx.raw_key, &ctx.temp) {
            Ok(dec) => out.push(dec),
            Err(error) => {
                let file = db.file_name().and_then(|name| name.to_str()).unwrap_or("?");
                errors.push(format!("{file}: {error}"));
            }
        }
    }
    ctx.message_dbs_dec = out;
    ctx.message_errors = errors;
    ctx.messages_ready = true;
}

/// Run blocking work (FFI, decryption, SQLite) off the UI thread.
async fn blocking<T, F>(f: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Other(format!("后台任务失败：{e}")))?
}

/// Read chat records from whichever backend is active.
fn read_chat_from(
    shared: &SharedBackend,
    talker: &str,
    limit: usize,
    text_only: bool,
) -> AppResult<(String, Vec<ChatMessage>)> {
    let mut guard = lock_backend(shared)?;
    match guard.as_mut() {
        Some(Backend::Real(ctx)) => {
            ensure_messages(ctx);
            if ctx.message_dbs_dec.is_empty() {
                let detail = if ctx.message_dbs_enc.is_empty() {
                    "未找到 message_*.db / biz_message_*.db".to_string()
                } else {
                    ctx.message_errors
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("；")
                };
                return Err(AppError::Decrypt(format!(
                    "未能解密任何消息数据库：{detail}"
                )));
            }
            let msgs = dbread::read_chat(&ctx.message_dbs_dec, talker, limit, text_only)?;
            Ok((ctx.wxid.clone(), msgs))
        }
        Some(Backend::Mock) => {
            let mut msgs = mock::chat(talker);
            if text_only {
                msgs.retain(|m| m.msg_type == 1);
            }
            // Keep the newest `limit` records, like the real path does.
            if msgs.len() > limit {
                msgs.drain(..msgs.len() - limit);
            }
            Ok(("mock".to_string(), msgs))
        }
        None => Err(AppError::NotInitialized),
    }
}

#[tauri::command]
async fn init_wechat(state: tauri::State<'_, AppState>) -> Result<WxStatus, AppError> {
    let result = tauri::async_runtime::spawn_blocking(try_init_real)
        .await
        .map_err(|e| AppError::Other(format!("初始化任务失败：{e}")))?;

    let (backend, status) = match result {
        Ok(ctx) => {
            let wxid = ctx.wxid.clone();
            (
                Backend::Real(ctx),
                WxStatus {
                    ready: true,
                    mode: "real".into(),
                    message: "已连接本地微信数据".into(),
                    wxid: Some(wxid),
                    detail: None,
                },
            )
        }
        Err(e) => (
            Backend::Mock,
            WxStatus {
                ready: true,
                mode: "mock".into(),
                message: format!("{e}（已切换到演示数据）"),
                wxid: None,
                detail: Some(e.to_string()),
            },
        ),
    };
    *lock_backend(&state.backend)? = Some(backend);
    Ok(status)
}

#[tauri::command]
async fn get_sessions(state: tauri::State<'_, AppState>) -> AppResult<Vec<Session>> {
    let shared = state.backend.clone();
    blocking(move || {
        let guard = lock_backend(&shared)?;
        match guard.as_ref() {
            Some(Backend::Real(ctx)) => {
                dbread::read_sessions(&ctx.session_db_dec, ctx.contact_db_dec.as_deref())
            }
            Some(Backend::Mock) => Ok(mock::sessions()),
            None => Err(AppError::NotInitialized),
        }
    })
    .await
}

#[tauri::command]
async fn build_wordcloud(
    state: tauri::State<'_, AppState>,
    talker: String,
    limit: usize,
    top_n: usize,
) -> AppResult<Vec<WordFreq>> {
    let shared = state.backend.clone();
    blocking(move || {
        let (_, msgs) = read_chat_from(&shared, &talker, limit, true)?;
        let texts: Vec<String> = msgs.into_iter().map(|m| m.text).collect();
        Ok(wordcloud::top_words(&texts, top_n))
    })
    .await
}

/// Save the selected conversation to `Documents\微信词云导出\<name>_<stamp>.json`.
#[tauri::command]
async fn export_chat_json(
    state: tauri::State<'_, AppState>,
    talker: String,
    display_name: String,
    limit: usize,
) -> AppResult<ExportResult> {
    let shared = state.backend.clone();
    blocking(move || {
        let (wxid, msgs) = read_chat_from(&shared, &talker, limit, false)?;
        let result = export::export_json(&wxid, &talker, &display_name, &msgs)?;
        export::reveal_in_explorer(std::path::Path::new(&result.path));
        Ok(result)
    })
    .await
}

/// Step-by-step check of the real-reading pipeline, so a user can see exactly
/// which stage keeps the app in demo mode.
#[tauri::command]
async fn diagnose() -> AppResult<Diagnostics> {
    blocking(|| {
        let mut d = Diagnostics::default();

        let account = match locate::find_account() {
            Ok(a) => {
                d.push(
                    "微信账号目录",
                    true,
                    format!(
                        "wxid={} / session.db={} / message DB {} 个",
                        a.wxid,
                        a.session_db.display(),
                        a.message_dbs.len()
                    ),
                );
                Some(a)
            }
            Err(e) => {
                d.push("微信账号目录", false, e.to_string());
                None
            }
        };

        let dll = match config::find_wx_key_dll() {
            Ok(p) => {
                d.push("wx_key.dll", true, p.display().to_string());
                Some(p)
            }
            Err(e) => {
                d.push("wx_key.dll", false, e.to_string());
                None
            }
        };

        let key = match &dll {
            Some(p) => match ffi_key::get_db_key(p, KEY_TIMEOUT) {
                Ok(k) => {
                    d.push("提取数据库密钥", true, "已获取 64 位十六进制密钥");
                    Some(k)
                }
                Err(e) => {
                    d.push("提取数据库密钥", false, e.to_string());
                    None
                }
            },
            None => {
                d.push("提取数据库密钥", false, "缺少 wx_key.dll，已跳过");
                None
            }
        };

        match (&account, &key) {
            (Some(a), Some(k)) => match parse_key(k).and_then(|rk| dbcrypt::detect(&a.session_db, &rk)) {
                Ok(_) => d.push("解密参数探测", true, "page1 HMAC 校验通过，可解密 session.db"),
                Err(e) => d.push(
                    "解密参数探测",
                    false,
                    format!("{e}（微信版本的加密参数可能不在探测范围内）"),
                ),
            },
            _ => d.push("解密参数探测", false, "缺少账号目录或密钥，已跳过"),
        }

        Ok(d)
    })
    .await
}

fn collect_schema(shared: &SharedBackend) -> AppResult<std::collections::BTreeMap<String, Vec<String>>> {
    let mut guard = lock_backend(shared)?;
    match guard.as_mut() {
        Some(Backend::Real(ctx)) => {
            ensure_messages(ctx);
            let mut dbs: Vec<&std::path::Path> = vec![ctx.session_db_dec.as_path()];
            if let Some(c) = &ctx.contact_db_dec {
                dbs.push(c.as_path());
            }
            for m in &ctx.message_dbs_dec {
                dbs.push(m.as_path());
            }
            Ok(dbread::dump_schema(&dbs))
        }
        Some(Backend::Mock) => Err(AppError::Other(
            "演示模式没有真实表结构；请先让状态栏显示『已连接微信数据』".into(),
        )),
        None => Err(AppError::NotInitialized),
    }
}

/// Diagnostic: dump decrypted DB schema (helps when a WeChat build differs).
#[tauri::command]
async fn dump_schema(
    state: tauri::State<'_, AppState>,
) -> AppResult<std::collections::BTreeMap<String, Vec<String>>> {
    let shared = state.backend.clone();
    blocking(move || collect_schema(&shared)).await
}

/// Save the decrypted DBs' schema to a JSON file, so a mismatching WeChat build
/// can be reported and `dbread`'s column candidates recalibrated.
#[tauri::command]
async fn export_schema(state: tauri::State<'_, AppState>) -> AppResult<ExportResult> {
    let shared = state.backend.clone();
    blocking(move || {
        let schema = collect_schema(&shared)?;
        let result = export::export_schema(&schema)?;
        export::reveal_in_explorer(std::path::Path::new(&result.path));
        Ok(result)
    })
    .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            init_wechat,
            get_sessions,
            build_wordcloud,
            export_chat_json,
            diagnose,
            dump_schema,
            export_schema
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
