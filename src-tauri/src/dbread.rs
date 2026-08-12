//! Read sessions and messages from decrypted (plaintext) WeChat 4.x DBs.
//!
//! - Sessions: introspect session.db, pick the table exposing a username-like
//!   column; join contact.db for display names.
//! - Messages: WeChat shards messages into `Msg_<md5(talker)>` tables across
//!   several message_*.db files; text messages are `local_type == 1` with the
//!   body in `message_content` (fallback `compress_content`, which is zstd).

use crate::error::AppResult;
use crate::model::{classify_talker, type_label, type_placeholder, ChatMessage, Session};
use crate::timefmt;
use md5::{Digest, Md5};
use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn open_ro(path: &Path) -> AppResult<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}

fn tables(conn: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")
    {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
            out.extend(rows.filter_map(Result::ok));
        }
    }
    out
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let sql = format!("PRAGMA table_info(\"{}\")", table.replace('"', ""));
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) {
            out.extend(rows.filter_map(Result::ok));
        }
    }
    out
}

fn pick<'a>(cols: &'a [String], cands: &[&str]) -> Option<&'a String> {
    for c in cands {
        if let Some(f) = cols.iter().find(|x| x.eq_ignore_ascii_case(c)) {
            return Some(f);
        }
    }
    for c in cands {
        if let Some(f) = cols.iter().find(|x| x.to_lowercase().contains(&c.to_lowercase())) {
            return Some(f);
        }
    }
    None
}

const TALKER_COLS: &[&str] = &["username", "user_name", "userName", "talker", "wxid", "strUsrName"];
const NAME_COLS: &[&str] = &["remark", "nickname", "nick_name", "displayName", "alias"];
const TIME_COLS: &[&str] = &["sort_timestamp", "last_timestamp", "sort_time", "create_time", "update_time"];
const CONTENT_COLS: &[&str] = &["message_content", "messageContent", "StrContent", "content"];
const COMPRESS_COLS: &[&str] = &["compress_content", "compressContent", "CompressContent"];
const TYPE_COLS: &[&str] = &["local_type", "localType", "type", "msg_type"];
const MSG_TIME_COLS: &[&str] = &["create_time", "createTime", "CreateTime", "timestamp", "sequence"];
const SENDER_COLS: &[&str] = &["real_sender_id", "sender_username", "sender_wxid", "talker_id", "sender"];
const SELF_COLS: &[&str] = &["is_sender", "is_self", "isSend", "IsSender"];

fn md5_hex(s: &str) -> String {
    let mut h = Md5::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

/// Read the session list (talker + display name + last time).
pub fn read_sessions(session_db: &Path, contact_db: Option<&Path>) -> AppResult<Vec<Session>> {
    let conn = open_ro(session_db)?;

    // Find the actual conversation table. `SessionUnreadListTable_*` also has
    // username_id + create_time, so "last matching table wins" incorrectly
    // selects it on current WeChat 4.x builds.
    let mut chosen: Option<(i32, String, String, Option<String>, Option<String>)> = None;
    for t in tables(&conn) {
        let cols = columns(&conn, &t);
        if let Some(tk) = pick(&cols, TALKER_COLS) {
            let nm = pick(&cols, NAME_COLS).cloned();
            let tm = pick(&cols, TIME_COLS).cloned();
            let exact_talker = TALKER_COLS.iter().any(|candidate| tk.eq_ignore_ascii_case(candidate));
            let score = if t.eq_ignore_ascii_case("SessionTable") { 100 } else { 0 }
                + if exact_talker { 20 } else { 0 }
                + if tm.is_some() { 10 } else { 0 }
                + if nm.is_some() { 1 } else { 0 };
            if chosen.as_ref().map(|(best, ..)| score > *best).unwrap_or(true) {
                chosen = Some((score, t.clone(), tk.clone(), nm, tm));
            }
        }
    }

    let contact_names = contact_db
        .and_then(|p| read_contact_names(p).ok())
        .unwrap_or_default();

    let Some((_, table, talker_col, name_col, time_col)) = chosen else {
        return Ok(Vec::new());
    };
    let session_ids = load_name2id(&conn);

    let name_sel = name_col.map(|c| format!("\"{c}\"")).unwrap_or_else(|| "NULL".into());
    let time_sel = time_col.map(|c| format!("\"{c}\"")).unwrap_or_else(|| "0".into());
    let sql = format!("SELECT \"{talker_col}\", {name_sel}, {time_sel} FROM \"{table}\"");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| {
        let talker: Value = r.get(0).unwrap_or(Value::Null);
        let name: Value = r.get(1).unwrap_or(Value::Null);
        let ts: Value = r.get(2).unwrap_or(Value::Null);
        Ok((talker, name, ts))
    })?;

    let mut out = Vec::new();
    for row in rows.filter_map(Result::ok) {
        let (talker_value, name_value, ts_value) = row;
        let talker = value_to_sender(&talker_value, &session_ids);
        if talker.is_empty() {
            continue;
        }
        let name = decode_value(name_value);
        let display = (!name.is_empty())
            .then_some(name)
            .or_else(|| contact_names.get(&talker).cloned())
            .unwrap_or_else(|| talker.clone());
        out.push(Session {
            kind: classify_talker(&talker),
            talker,
            display_name: display,
            last_timestamp: value_to_i64(&ts_value),
        });
    }
    out.sort_by(|a, b| b.last_timestamp.cmp(&a.last_timestamp));
    Ok(out)
}

fn read_contact_names(contact_db: &Path) -> AppResult<BTreeMap<String, String>> {
    let conn = open_ro(contact_db)?;
    let contact_ids = load_name2id(&conn);
    let mut out = BTreeMap::new();
    for t in tables(&conn) {
        let cols = columns(&conn, &t);
        let Some(tk) = pick(&cols, TALKER_COLS) else {
            continue;
        };
        let name_cols: Vec<&String> = NAME_COLS
            .iter()
            .filter_map(|candidate| cols.iter().find(|col| col.eq_ignore_ascii_case(candidate)))
            .collect();
        if name_cols.is_empty() {
            continue;
        }
        let name_expr = name_cols
            .iter()
            .map(|col| format!("NULLIF(CAST(\"{col}\" AS TEXT), '')"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT \"{tk}\", COALESCE({name_expr}, '') FROM \"{t}\"");
        if let Ok(mut stmt) = conn.prepare(&sql) {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, Value>(0).unwrap_or(Value::Null),
                    r.get::<_, String>(1).unwrap_or_default(),
                ))
            }) {
                for (key, v) in rows.filter_map(Result::ok) {
                    let k = value_to_sender(&key, &contact_ids);
                    if !k.is_empty() && !v.is_empty() {
                        out.entry(k).or_insert(v);
                    }
                }
            }
        }
    }
    Ok(out)
}

fn decode_bytes(b: &[u8]) -> String {
    // zstd magic 0xFD2FB528 => LE bytes 28 B5 2F FD
    if b.len() >= 4 && b[0] == 0x28 && b[1] == 0xB5 && b[2] == 0x2F && b[3] == 0xFD {
        if let Ok(d) = zstd::stream::decode_all(b) {
            return String::from_utf8_lossy(&d).into_owned();
        }
    }
    String::from_utf8_lossy(b).into_owned()
}

fn decode_value(v: Value) -> String {
    match v {
        Value::Text(s) => s,
        Value::Blob(b) => decode_bytes(&b),
        _ => String::new(),
    }
}

/// Whether a table name is the `Msg_<md5(talker)>` shard for this talker.
/// Builds differ on how much of the hash they keep, so accept prefixes.
fn is_talker_table(table: &str, hash32: &str, hash16: &str) -> bool {
    let lname = table.to_lowercase();
    let Some(suffix) = lname.strip_prefix("msg_") else {
        return false;
    };
    suffix == hash32 || suffix.starts_with(hash16) || hash32.starts_with(suffix)
}

/// WeChat 4.x stores the group speaker as an integer id resolved through a
/// `Name2Id` table (rowid -> username). Best effort: absent table means we keep
/// the raw id, which is still a usable sender identifier.
fn load_name2id(conn: &Connection) -> BTreeMap<i64, String> {
    let mut out = BTreeMap::new();
    for t in tables(conn) {
        if !t.to_lowercase().contains("name2id") {
            continue;
        }
        let cols = columns(conn, &t);
        let name_col = pick(&cols, TALKER_COLS).cloned().or_else(|| cols.first().cloned());
        let Some(name_col) = name_col else { continue };
        let sql = format!("SELECT rowid, \"{name_col}\" FROM \"{t}\"");
        if let Ok(mut stmt) = conn.prepare(&sql) {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok((r.get::<_, i64>(0).unwrap_or(0), r.get::<_, String>(1).unwrap_or_default()))
            }) {
                for (id, name) in rows.filter_map(Result::ok) {
                    if !name.is_empty() {
                        out.entry(id).or_insert(name);
                    }
                }
            }
        }
    }
    out
}

/// Group text bodies are stored as `wxid_xxx:\n<text>`; split that apart so the
/// word cloud never eats the wxid and the export gets a real speaker.
fn split_group_prefix(body: &str) -> (Option<String>, &str) {
    let head: String = body.chars().take(80).collect();
    let Some(colon) = head.find(':') else {
        return (None, body);
    };
    let (prefix, rest) = body.split_at(colon);
    let rest = &rest[1..];
    let is_id = !prefix.is_empty()
        && prefix.len() >= 4
        && prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@'));
    if is_id && (rest.starts_with('\n') || rest.starts_with("\r\n")) {
        (Some(prefix.to_string()), rest.trim_start_matches(['\r', '\n']))
    } else {
        (None, body)
    }
}

fn value_to_i64(v: &Value) -> i64 {
    match v {
        Value::Integer(i) => *i,
        Value::Real(f) => *f as i64,
        Value::Text(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn value_to_sender(v: &Value, name2id: &BTreeMap<i64, String>) -> String {
    match v {
        Value::Text(s) => s.clone(),
        Value::Integer(i) => name2id.get(i).cloned().unwrap_or_else(|| i.to_string()),
        Value::Real(f) => {
            let i = *f as i64;
            name2id.get(&i).cloned().unwrap_or_else(|| i.to_string())
        }
        _ => String::new(),
    }
}

/// Read chat records for a talker across all message DBs.
///
/// Returns the **newest** `limit` records, ordered oldest -> newest, so both the
/// export file and the word cloud reflect recent conversation. With
/// `text_only`, non-text messages are skipped entirely (word-cloud path);
/// otherwise they are kept with a `[图片]`-style placeholder body.
pub fn read_chat(
    message_dbs: &[PathBuf],
    talker: &str,
    limit: usize,
    text_only: bool,
) -> AppResult<Vec<ChatMessage>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let hash32 = md5_hex(talker).to_lowercase();
    let hash16 = &hash32[..16];
    let is_group = talker.ends_with("@chatroom");
    let mut out: Vec<ChatMessage> = Vec::new();

    for db in message_dbs {
        let Ok(conn) = open_ro(db) else {
            continue;
        };
        let mut name2id: Option<BTreeMap<i64, String>> = None;

        for t in tables(&conn) {
            if !is_talker_table(&t, &hash32, hash16) {
                continue;
            }
            let cols = columns(&conn, &t);
            let Some(content_col) = pick(&cols, CONTENT_COLS).cloned() else {
                continue;
            };
            let compress_col = pick(&cols, COMPRESS_COLS).cloned();
            let type_col = pick(&cols, TYPE_COLS).cloned();
            let time_col = pick(&cols, MSG_TIME_COLS).cloned();
            let self_col = pick(&cols, SELF_COLS).cloned();
            // `pick` falls back to substring matching, which can land on the
            // same column twice (e.g. "is_sender" for both); don't double-use it.
            let sender_col = pick(&cols, SENDER_COLS)
                .cloned()
                .filter(|c| Some(c) != self_col.as_ref());

            let sel = |c: &Option<String>| {
                c.as_ref().map(|c| format!("\"{c}\"")).unwrap_or_else(|| "NULL".into())
            };
            let where_sql = match (&type_col, text_only) {
                (Some(c), true) => format!("WHERE \"{c}\" = 1"),
                _ => String::new(),
            };
            let order_sql = time_col
                .as_ref()
                .map(|c| format!("ORDER BY \"{c}\" DESC"))
                .unwrap_or_else(|| "ORDER BY rowid DESC".into());
            let sql = format!(
                "SELECT \"{content_col}\", {}, {}, {}, {}, {} FROM \"{t}\" {where_sql} {order_sql} LIMIT {limit}",
                sel(&compress_col),
                sel(&type_col),
                sel(&time_col),
                sel(&sender_col),
                sel(&self_col),
            );

            let Ok(mut stmt) = conn.prepare(&sql) else {
                continue;
            };
            let Ok(rows) = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, Value>(0).unwrap_or(Value::Null),
                    r.get::<_, Value>(1).unwrap_or(Value::Null),
                    r.get::<_, Value>(2).unwrap_or(Value::Null),
                    r.get::<_, Value>(3).unwrap_or(Value::Null),
                    r.get::<_, Value>(4).unwrap_or(Value::Null),
                    r.get::<_, Value>(5).unwrap_or(Value::Null),
                ))
            }) else {
                continue;
            };

            let ids = name2id.get_or_insert_with(|| load_name2id(&conn));

            for (content, compress, ty, time, sender, is_self) in rows.filter_map(Result::ok) {
                let msg_type = match &ty {
                    Value::Null => 1,
                    v => value_to_i64(v),
                };
                if text_only && msg_type != 1 {
                    continue;
                }

                let mut body = decode_value(compress);
                if body.trim().is_empty() {
                    body = decode_value(content);
                }
                let is_self = value_to_i64(&is_self) != 0;
                let mut sender = value_to_sender(&sender, ids);

                if is_group {
                    let (prefix, rest) = split_group_prefix(&body);
                    if let Some(p) = prefix {
                        if sender.is_empty() || sender.parse::<i64>().is_ok() {
                            sender = p;
                        }
                        body = rest.to_string();
                    }
                }
                if sender.is_empty() {
                    sender = if is_self { "self".into() } else { talker.to_string() };
                }

                let text = if msg_type == 1 {
                    body.trim().to_string()
                } else if body.trim().is_empty() {
                    type_placeholder(msg_type)
                } else {
                    body.trim().to_string()
                };
                if text_only && text.is_empty() {
                    continue;
                }

                let timestamp = timefmt::normalize_seconds(value_to_i64(&time));
                out.push(ChatMessage {
                    timestamp,
                    time_text: timefmt::format_local(timestamp),
                    sender,
                    is_self,
                    msg_type,
                    type_label: type_label(msg_type).to_string(),
                    text,
                });
            }
        }
    }

    // Shards span multiple DB files: keep the newest `limit`, emit chronologically.
    out.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    out.truncate(limit);
    out.reverse();
    Ok(out)
}

/// Diagnostic: dump table -> columns for decrypted DBs (used when schema differs).
pub fn dump_schema(dbs: &[&Path]) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for db in dbs {
        let file = db.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string();
        if let Ok(conn) = open_ro(db) {
            for t in tables(&conn) {
                out.insert(format!("{file}::{t}"), columns(&conn, &t));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_only_this_talkers_shard() {
        let talker = "wxid_abc123";
        let h = md5_hex(talker).to_lowercase();
        assert!(is_talker_table(&format!("Msg_{h}"), &h, &h[..16]));
        assert!(is_talker_table(&format!("MSG_{}", &h[..16]), &h, &h[..16]));
        assert!(!is_talker_table("Msg_0123456789abcdef0123456789abcdef", &h, &h[..16]));
        assert!(!is_talker_table("Name2Id", &h, &h[..16]));
        assert!(!is_talker_table("session", &h, &h[..16]));
    }

    #[test]
    fn splits_group_sender_prefix() {
        let (sender, body) = split_group_prefix("wxid_abc123:\n周末去爬山");
        assert_eq!(sender.as_deref(), Some("wxid_abc123"));
        assert_eq!(body, "周末去爬山");

        // Plain text containing a colon must stay intact.
        let (sender, body) = split_group_prefix("时间: 明天下午三点");
        assert_eq!(sender, None);
        assert_eq!(body, "时间: 明天下午三点");

        let (sender, body) = split_group_prefix("没有冒号的消息");
        assert_eq!(sender, None);
        assert_eq!(body, "没有冒号的消息");
    }

    /// Build a DB shaped like WeChat 4.x so the reader is testable offline.
    fn make_message_db(dir: &Path, talker: &str) -> PathBuf {
        let path = dir.join("message_0.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE Name2Id(user_name TEXT);
             INSERT INTO Name2Id(rowid, user_name) VALUES (7, 'wxid_speaker');",
        )
        .unwrap();
        let table = format!("Msg_{}", md5_hex(talker).to_lowercase());
        conn.execute_batch(&format!(
            "CREATE TABLE \"{table}\"(
                local_id INTEGER PRIMARY KEY,
                local_type INTEGER,
                create_time INTEGER,
                real_sender_id INTEGER,
                is_sender INTEGER,
                message_content TEXT,
                compress_content BLOB
             );
             INSERT INTO \"{table}\"(local_type, create_time, real_sender_id, is_sender, message_content)
             VALUES (1, 1700000200, 7, 0, '第二条文本'),
                    (1, 1700000100, 0, 1, '第一条文本'),
                    (3, 1700000300, 7, 0, NULL),
                    (1, 1700000400, 7, 0, '最新一条文本');"
        ))
        .unwrap();
        drop(conn);
        path
    }

    #[test]
    fn reads_records_chronologically_with_labels() {
        let dir = std::env::temp_dir().join(format!("wc_dbread_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let talker = "wxid_friend";
        let dbs = vec![make_message_db(&dir, talker)];

        let all = read_chat(&dbs, talker, 100, false).unwrap();
        assert_eq!(all.len(), 4);
        // Oldest first.
        assert_eq!(all[0].text, "第一条文本");
        assert_eq!(all[3].text, "最新一条文本");
        assert!(all.windows(2).all(|w| w[0].timestamp <= w[1].timestamp));
        // Self flag and sender resolution through Name2Id.
        assert!(all[0].is_self);
        assert_eq!(all[1].sender, "wxid_speaker");
        // The image row keeps a placeholder body, never a media file.
        let image = all.iter().find(|m| m.msg_type == 3).unwrap();
        assert_eq!(image.type_label, "图片");
        assert_eq!(image.text, "[图片]");
        assert!(!all[0].time_text.is_empty());

        // Word-cloud path (text_only) drops non-text rows.
        let texts = read_chat(&dbs, talker, 100, true).unwrap();
        assert_eq!(texts.len(), 3);
        assert!(texts.iter().all(|m| m.msg_type == 1 && !m.text.contains("[图片]")));

        // A limit keeps the newest records, still ordered oldest -> newest.
        let recent = read_chat(&dbs, talker, 2, false).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[1].text, "最新一条文本");

        assert!(read_chat(&dbs, "wxid_nobody", 100, false).unwrap().is_empty());
        assert!(read_chat(&dbs, talker, 0, false).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn strips_the_speaker_prefix_in_group_messages() {
        let dir = std::env::temp_dir().join(format!("wc_dbread_group_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let talker = "family@chatroom";
        let path = dir.join("message_0.db");
        let conn = Connection::open(&path).unwrap();
        let table = format!("Msg_{}", md5_hex(talker).to_lowercase());
        conn.execute_batch(&format!(
            "CREATE TABLE \"{table}\"(
                local_type INTEGER, create_time INTEGER, message_content TEXT
             );
             INSERT INTO \"{table}\" VALUES (1, 1700000100, 'wxid_uncle:
周末一起爬山');"
        ))
        .unwrap();
        drop(conn);

        let msgs = read_chat(&[path], talker, 10, true).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].sender, "wxid_uncle");
        assert_eq!(msgs[0].text, "周末一起爬山");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_wechat4_sessions_with_numeric_name2id_keys() {
        let dir = std::env::temp_dir().join(format!(
            "wc_dbread_sessions_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let session_path = dir.join("session.db");
        let session = Connection::open(&session_path).unwrap();
        session
            .execute_batch(
                "CREATE TABLE Name2Id(user_name TEXT);
                 INSERT INTO Name2Id(rowid, user_name)
                   VALUES (1, 'wxid_friend'), (2, 'family@chatroom');
                 CREATE TABLE SessionTable(username INTEGER, sort_timestamp INTEGER);
                 INSERT INTO SessionTable VALUES (1, 1700000100), (2, 1700000200);
                 CREATE TABLE SessionUnreadListTable_1(
                   username_id INTEGER, server_id INTEGER, create_time INTEGER
                 );
                 INSERT INTO SessionUnreadListTable_1 VALUES (1, 99, 1700000300);",
            )
            .unwrap();
        drop(session);

        let contact_path = dir.join("contact.db");
        let contact = Connection::open(&contact_path).unwrap();
        contact
            .execute_batch(
                "CREATE TABLE name2id(username TEXT);
                 INSERT INTO name2id(rowid, username)
                   VALUES (1, 'wxid_friend'), (2, 'family@chatroom');
                 CREATE TABLE contact(
                   username INTEGER, remark TEXT, nick_name TEXT, alias TEXT
                 );
                 INSERT INTO contact VALUES
                   (1, '', '好友昵称', ''),
                   (2, '家庭群备注', '家庭群昵称', '');",
            )
            .unwrap();
        drop(contact);

        let sessions = read_sessions(&session_path, Some(&contact_path)).unwrap();
        assert_eq!(sessions.len(), 2);
        // SessionTable must win over SessionUnreadListTable_1, and newest first.
        assert_eq!(sessions[0].talker, "family@chatroom");
        assert_eq!(sessions[0].display_name, "家庭群备注");
        assert_eq!(sessions[0].kind, "group");
        assert_eq!(sessions[1].talker, "wxid_friend");
        // Empty remark falls back to nick_name.
        assert_eq!(sessions[1].display_name, "好友昵称");

        let _ = std::fs::remove_dir_all(dir);
    }
}
