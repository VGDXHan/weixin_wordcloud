//! Locate the WeChat 4.x account data directory and its databases.
//! Layout: <root>\xwechat_files\<wxid>\db_storage\{session,message,contact}\*.db
//! The root may be relocated by the user, so scan default + all fixed drives.

use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct WxAccount {
    pub wxid: String,
    pub session_db: PathBuf,
    pub message_dbs: Vec<PathBuf>,
    pub contact_db: Option<PathBuf>,
}

fn roots() -> Vec<PathBuf> {
    let mut v = Vec::new();

    if let Some(dir) = std::env::var_os("WECHAT_DATA_DIR") {
        v.push(PathBuf::from(dir));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let p = PathBuf::from(profile);
        v.push(p.join("Documents").join("xwechat_files"));
        v.push(p.join("Documents").join("WeChat Files"));
    }
    // Custom locations: scan each fixed drive root + first level for xwechat_files.
    for letter in b'C'..=b'Z' {
        let drive = PathBuf::from(format!("{}:\\", letter as char));
        if !drive.is_dir() {
            continue;
        }
        let direct = drive.join("xwechat_files");
        if direct.is_dir() {
            v.push(direct);
        }
        if let Ok(entries) = std::fs::read_dir(&drive) {
            for e in entries.flatten() {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let cand = e.path().join("xwechat_files");
                    if cand.is_dir() {
                        v.push(cand);
                    }
                }
            }
        }
    }

    let mut seen = std::collections::HashSet::new();
    v.retain(|p| seen.insert(p.clone()));
    v
}

fn session_db_in(account_dir: &Path) -> Option<PathBuf> {
    [
        account_dir.join("db_storage").join("session").join("session.db"),
        account_dir.join("db_storage").join("session.db"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

fn list_message_db_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            // media_*.db can be very large and never contains Msg_* tables.
            // Keep personal and official-account message shards only.
            let shard = stem
                .strip_prefix("message_")
                .or_else(|| stem.strip_prefix("biz_message_"));
            let is_message = shard
                .map(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
                .unwrap_or(false);
            if is_message && p.extension().and_then(|s| s.to_str()) == Some("db") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Find the most recently used account (largest session.db mtime) with its DBs.
pub fn find_account() -> AppResult<WxAccount> {
    let mut best: Option<(WxAccount, std::time::SystemTime)> = None;

    for root in roots() {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.eq_ignore_ascii_case("all_users") || !name.contains('_') {
                continue;
            }
            let Some(session_db) = session_db_in(&dir) else {
                continue;
            };
            let db_storage = dir.join("db_storage");
            let message_dbs = list_message_db_files(&db_storage.join("message"));
            let contact_db = {
                let c = db_storage.join("contact").join("contact.db");
                c.is_file().then_some(c)
            };
            let mtime = std::fs::metadata(&session_db)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            let acc = WxAccount {
                wxid: name,
                session_db,
                message_dbs,
                contact_db,
            };
            if best.as_ref().map(|(_, t)| mtime > *t).unwrap_or(true) {
                best = Some((acc, mtime));
            }
        }
    }

    best.map(|(a, _)| a).ok_or_else(|| {
        AppError::DataDirNotFound(
            "未找到含 session.db 的微信账号目录，请确认微信 4.x 已登录".into(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_numbered_message_shards() {
        let dir = std::env::temp_dir().join(format!(
            "weixin_wordcloud_locate_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "message_0.db",
            "message_12.db",
            "biz_message_2.db",
            "media_0.db",
            "biz_media_0.db",
            "message_fts.db",
            "message_resource.db",
            "message_.db",
            "message_3.wal",
        ] {
            std::fs::write(dir.join(name), []).unwrap();
        }

        let names: Vec<String> = list_message_db_files(&dir)
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["biz_message_2.db", "message_0.db", "message_12.db"]
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
