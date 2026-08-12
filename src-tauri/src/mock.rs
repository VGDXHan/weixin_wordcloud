//! Mock data so the UI works end-to-end without a running WeChat.
//! Used automatically when the real reading pipeline is unavailable.

use crate::model::{classify_talker, type_label, ChatMessage, Session};
use crate::timefmt;

pub fn sessions() -> Vec<Session> {
    let raw = [
        ("wxid_demo_friend", "张三（演示）"),
        ("family_group@chatroom", "家庭群（演示）"),
        ("wxid_demo_colleague", "李四（演示）"),
        ("gh_demo_official", "某公众号（演示）"),
    ];
    raw.iter()
        .enumerate()
        .map(|(i, (talker, name))| Session {
            talker: talker.to_string(),
            display_name: name.to_string(),
            kind: classify_talker(talker),
            last_timestamp: 1_700_000_000 - i as i64 * 3600,
        })
        .collect()
}

pub fn messages(talker: &str) -> Vec<String> {
    let base = if talker.ends_with("@chatroom") {
        vec![
            "周末大家一起去爬山吧天气很好",
            "爬山之后可以一起吃饭聚餐",
            "记得带上水和零食路上补充能量",
            "上次爬山的照片太好看了风景真棒",
            "下周继续组织户外活动运动一下",
            "运动真的可以让人心情变好呀",
        ]
    } else {
        vec![
            "今天的项目进展怎么样了代码写完了吗",
            "项目差不多完成了在做最后的测试",
            "测试通过之后就可以上线部署了",
            "上线之前记得再检查一遍配置文件",
            "配置没问题的话晚上就能发布版本",
            "周末一起打球放松一下运动运动",
            "打球之后去吃火锅怎么样火锅真香",
        ]
    };
    // Repeat a bit so word frequencies are more visible in the demo.
    base.iter()
        .cycle()
        .take(base.len() * 4)
        .map(|s| s.to_string())
        .collect()
}

/// Structured demo records so the JSON export is exercisable without WeChat.
/// Includes one non-text message to show how placeholders are exported.
pub fn chat(talker: &str) -> Vec<ChatMessage> {
    let start = 1_700_000_000i64;
    let texts = messages(talker);
    let mut out: Vec<ChatMessage> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let ts = start + i as i64 * 60;
            let is_self = i % 2 == 0;
            ChatMessage {
                timestamp: ts,
                time_text: timefmt::format_local(ts),
                sender: if is_self { "self（演示）".into() } else { talker.to_string() },
                is_self,
                msg_type: 1,
                type_label: type_label(1).to_string(),
                text: text.clone(),
            }
        })
        .collect();

    let ts = start + out.len() as i64 * 60;
    out.push(ChatMessage {
        timestamp: ts,
        time_text: timefmt::format_local(ts),
        sender: talker.to_string(),
        is_self: false,
        msg_type: 3,
        type_label: type_label(3).to_string(),
        text: "[图片]".into(),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Demo mode has to carry the whole feature set end to end, since it is the
    /// only path verifiable without a logged-in WeChat.
    #[test]
    fn demo_mode_supports_export_and_wordcloud() {
        let talker = "family_group@chatroom";
        let chat = chat(talker);
        assert!(chat.len() > 5);
        assert!(chat.windows(2).all(|w| w[0].timestamp <= w[1].timestamp));
        assert!(chat.iter().any(|m| m.is_self), "demo needs messages from self");
        assert!(chat.iter().all(|m| !m.time_text.is_empty()));
        assert!(chat.iter().any(|m| m.msg_type != 1), "demo needs a non-text message");

        let res = crate::export::export_json("mock", talker, "家庭群（演示）", &chat).unwrap();
        assert_eq!(res.count, chat.len());
        let text = std::fs::read_to_string(&res.path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["talker"], talker);
        assert_eq!(v["messages"].as_array().unwrap().len(), chat.len());
        let _ = std::fs::remove_file(&res.path);

        // The word cloud consumes text messages only.
        let texts: Vec<String> = chat
            .iter()
            .filter(|m| m.msg_type == 1)
            .map(|m| m.text.clone())
            .collect();
        let freqs = crate::wordcloud::top_words(&texts, 20);
        assert!(!freqs.is_empty());
        assert!(freqs.iter().all(|f| !f.word.starts_with('[')));
    }

    #[test]
    fn demo_sessions_cover_every_kind() {
        let s = sessions();
        assert_eq!(s.len(), 4);
        assert!(s.iter().any(|x| x.kind == "group"));
        assert!(s.iter().any(|x| x.kind == "official"));
        assert!(s.iter().any(|x| x.kind == "friend"));
    }
}
