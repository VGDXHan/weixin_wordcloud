//! Save a conversation to a local JSON file.
//!
//! Everything happens in the Rust process with `std::fs`, so no `tauri-plugin-fs`
//! / `dialog` is needed and `capabilities` stays at `core:default`.
//! Media files are never exported; non-text messages keep a placeholder body.

use crate::error::{AppError, AppResult};
use crate::model::{ChatMessage, ExportResult};
use crate::timefmt;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const EXPORT_DIR_NAME: &str = "微信词云导出";
const MAX_STEM_CHARS: usize = 60;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile<'a> {
    wxid: &'a str,
    talker: &'a str,
    display_name: &'a str,
    exported_at: String,
    count: usize,
    messages: &'a [ChatMessage],
}

/// Make a conversation name safe to use as a Windows file name.
pub fn sanitize_stem(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        let bad = matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            || (ch as u32) < 0x20;
        out.push(if bad { '_' } else { ch });
        if out.chars().count() >= MAX_STEM_CHARS {
            break;
        }
    }
    // Windows rejects trailing dots/spaces and reserves a few device names.
    let trimmed = out.trim().trim_end_matches('.').trim().to_string();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if trimmed.is_empty() {
        return "chat".to_string();
    }
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&trimmed)) {
        return format!("_{trimmed}");
    }
    trimmed
}

/// `Documents\微信词云导出` (falls back to the temp dir if there is no profile).
pub fn export_dir() -> PathBuf {
    let base = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|p| p.join("Documents"))
        .filter(|p| p.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    base.join(EXPORT_DIR_NAME)
}

/// Write `<stem>_<stamp>.json` into the export directory.
fn write_json<T: Serialize>(stem: &str, value: &T, count: usize) -> AppResult<ExportResult> {
    let dir = export_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::Other(format!("无法创建导出目录 {}：{e}", dir.display())))?;

    let path = dir.join(format!("{stem}_{}.json", timefmt::stamp_now()));
    let json = serde_json::to_vec_pretty(value)
        .map_err(|e| AppError::Other(format!("序列化导出内容失败：{e}")))?;
    std::fs::write(&path, &json)
        .map_err(|e| AppError::Other(format!("写入 {} 失败：{e}", path.display())))?;

    Ok(ExportResult {
        path: path.to_string_lossy().into_owned(),
        dir: dir.to_string_lossy().into_owned(),
        count,
    })
}

/// Write the conversation as JSON and return where it landed.
pub fn export_json(
    wxid: &str,
    talker: &str,
    display_name: &str,
    messages: &[ChatMessage],
) -> AppResult<ExportResult> {
    if messages.is_empty() {
        return Err(AppError::Other(
            "该会话没有可导出的消息（可能是消息表未命中，见 README 排查）".into(),
        ));
    }

    let stem = sanitize_stem(if display_name.trim().is_empty() {
        talker
    } else {
        display_name
    });
    let payload = ExportFile {
        wxid,
        talker,
        display_name,
        exported_at: timefmt::now_rfc3339(),
        count: messages.len(),
        messages,
    };
    write_json(&stem, &payload, messages.len())
}

/// Dump the decrypted DBs' table -> columns map, so a WeChat build whose schema
/// differs can be diagnosed (and `dbread`'s column candidates recalibrated).
pub fn export_schema(schema: &BTreeMap<String, Vec<String>>) -> AppResult<ExportResult> {
    if schema.is_empty() {
        return Err(AppError::Other("没有读到任何表结构（数据库可能未解密成功）".into()));
    }
    write_json("schema", schema, schema.len())
}

/// Open Explorer with the file selected. Best effort: never fails the export.
pub fn reveal_in_explorer(path: &Path) {
    let _ = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::type_label;

    fn msg(text: &str, ty: i64) -> ChatMessage {
        ChatMessage {
            timestamp: 1_700_000_000,
            time_text: timefmt::format_local(1_700_000_000),
            sender: "wxid_abc".into(),
            is_self: false,
            msg_type: ty,
            type_label: type_label(ty).to_string(),
            text: text.into(),
        }
    }

    #[test]
    fn sanitize_removes_illegal_characters() {
        assert_eq!(sanitize_stem("a/b\\c:d*e?f\"g<h>i|j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize_stem("正常群名"), "正常群名");
        assert_eq!(sanitize_stem("   "), "chat");
        assert_eq!(sanitize_stem("trailing..."), "trailing");
        assert_eq!(sanitize_stem("con"), "_con");
    }

    #[test]
    fn sanitize_limits_length_without_splitting_chars() {
        let long = "群".repeat(200);
        let s = sanitize_stem(&long);
        assert_eq!(s.chars().count(), MAX_STEM_CHARS);
    }

    #[test]
    fn json_uses_the_agreed_field_names() {
        let messages = vec![msg("你好", 1), msg("[图片]", 3)];
        let payload = ExportFile {
            wxid: "wxid_me",
            talker: "wxid_abc",
            display_name: "张三",
            exported_at: "2026-08-12T12:00:00+08:00".into(),
            count: messages.len(),
            messages: &messages,
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&payload).unwrap()).unwrap();

        for key in ["wxid", "talker", "displayName", "exportedAt", "count", "messages"] {
            assert!(v.get(key).is_some(), "missing top-level key {key}");
        }
        let m = &v["messages"][0];
        for key in ["timestamp", "timeText", "sender", "isSelf", "type", "typeLabel", "text"] {
            assert!(m.get(key).is_some(), "missing message key {key}");
        }
        assert_eq!(m["type"], 1);
        assert_eq!(m["typeLabel"], "文本");
        assert_eq!(v["messages"][1]["typeLabel"], "图片");
        assert_eq!(v["count"], 2);
    }

    #[test]
    fn export_writes_a_readable_file() {
        let messages = vec![msg("今天一起去看电影", 1)];
        let res = export_json("wxid_me", "wxid_abc", "张三", &messages).unwrap();
        let path = PathBuf::from(&res.path);
        assert!(path.is_file(), "export file should exist: {}", res.path);
        assert_eq!(res.count, 1);

        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["displayName"], "张三");
        assert_eq!(v["messages"][0]["text"], "今天一起去看电影");
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("张三_"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn export_rejects_an_empty_conversation() {
        assert!(export_json("wxid_me", "wxid_abc", "张三", &[]).is_err());
    }

    #[test]
    fn schema_export_writes_tables_and_columns() {
        let mut schema = BTreeMap::new();
        schema.insert(
            "dec_message_0.db::Msg_abc".to_string(),
            vec!["local_type".to_string(), "create_time".to_string()],
        );
        schema.insert("dec_session.db::SessionTable".to_string(), vec!["username".to_string()]);

        let res = export_schema(&schema).unwrap();
        assert_eq!(res.count, 2);
        let path = PathBuf::from(&res.path);
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("schema_"));

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["dec_message_0.db::Msg_abc"][0], "local_type");
        assert_eq!(v["dec_session.db::SessionTable"][0], "username");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn schema_export_rejects_an_empty_dump() {
        assert!(export_schema(&BTreeMap::new()).is_err());
    }
}
