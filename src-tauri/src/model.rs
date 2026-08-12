use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub talker: String,
    pub display_name: String,
    /// friend | group | official | other
    pub kind: String,
    pub last_timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordFreq {
    pub word: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WxStatus {
    pub ready: bool,
    /// real | mock
    pub mode: String,
    pub message: String,
    pub wxid: Option<String>,
    /// Failing stage + raw error, so the user can tell *why* real mode is off.
    pub detail: Option<String>,
}

/// One chat record as exported to JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    /// Unix seconds as stored by WeChat (0 when the column is missing).
    pub timestamp: i64,
    /// Human-readable local time derived from `timestamp`.
    pub time_text: String,
    /// Sender identifier; in groups the speaker, otherwise the talker or self.
    pub sender: String,
    pub is_self: bool,
    /// Raw WeChat `local_type`.
    #[serde(rename = "type")]
    pub msg_type: i64,
    pub type_label: String,
    /// Text body for text messages, else a placeholder such as `[图片]`.
    pub text: String,
}

/// Where an export landed on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub dir: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseStep {
    pub name: String,
    pub ok: bool,
    pub info: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub steps: Vec<DiagnoseStep>,
}

impl Diagnostics {
    pub fn push(&mut self, name: &str, ok: bool, info: impl Into<String>) {
        self.steps.push(DiagnoseStep {
            name: name.to_string(),
            ok,
            info: info.into(),
        });
    }
}

/// Human-readable label for WeChat's `local_type`.
pub fn type_label(t: i64) -> &'static str {
    match t {
        1 => "文本",
        3 => "图片",
        34 => "语音",
        42 => "名片",
        43 => "视频",
        47 => "动画表情",
        48 => "位置",
        49 => "链接/文件/转账",
        50 => "音视频通话",
        10000 => "系统消息",
        10002 => "撤回/系统提示",
        _ => "其他",
    }
}

/// Placeholder body used for non-text messages (we never export media files).
pub fn type_placeholder(t: i64) -> String {
    format!("[{}]", type_label(t))
}

pub fn classify_talker(talker: &str) -> String {
    if talker.ends_with("@chatroom") {
        "group".to_string()
    } else if talker.starts_with("gh_") {
        "official".to_string()
    } else if talker.starts_with("wxid_") || !talker.is_empty() {
        "friend".to_string()
    } else {
        "other".to_string()
    }
}
