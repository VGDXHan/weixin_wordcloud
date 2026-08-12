use crate::model::WordFreq;
use jieba_rs::Jieba;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

static JIEBA: Lazy<Jieba> = Lazy::new(Jieba::new);

static URL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"https?://\S+|www\.\S+").unwrap());

// Common Chinese stopwords + WeChat system placeholders. Kept compact on
// purpose; extend as needed.
static STOPWORDS: Lazy<std::collections::HashSet<&'static str>> = Lazy::new(|| {
    const WORDS: &[&str] = &[
        "的", "了", "在", "是", "我", "你", "他", "她", "它", "们", "这", "那",
        "有", "和", "就", "都", "而", "及", "与", "着", "或", "一个", "没有",
        "我们", "你们", "他们", "自己", "什么", "怎么", "这个", "那个", "这样",
        "那样", "现在", "可以", "但是", "不是", "还是", "因为", "所以", "如果",
        "然后", "已经", "这些", "那些", "知道", "觉得", "时候", "一下", "一样",
        "不会", "不能", "这么", "那么", "只是", "还有", "为了", "非常", "真的",
        "好的", "嗯嗯", "哈哈", "哈哈哈", "呵呵", "哦哦", "嗯", "啊", "吧",
        "呢", "吗", "哦", "哈", "呀", "嘛", "喔", "噢", "哎",
        // WeChat placeholders
        "图片", "表情", "语音", "视频", "动画表情", "位置", "文件", "链接",
        "转账", "红包", "拍了拍", "撤回了一条消息",
    ];
    WORDS.iter().copied().collect()
});

/// Compute Top-N word frequencies over a batch of chat text messages.
pub fn top_words(messages: &[String], top_n: usize) -> Vec<WordFreq> {
    let mut counts: HashMap<String, usize> = HashMap::new();

    for msg in messages {
        let cleaned = URL_RE.replace_all(msg, " ");
        for token in JIEBA.cut(&cleaned, true) {
            if is_meaningful(token) {
                *counts.entry(token.to_string()).or_insert(0) += 1;
            }
        }
    }

    let mut freqs: Vec<WordFreq> = counts
        .into_iter()
        .map(|(word, count)| WordFreq { word, count })
        .collect();

    // Descending by count, then by word for deterministic ordering on ties.
    freqs.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.word.cmp(&b.word)));
    freqs.truncate(top_n);
    freqs
}

fn is_meaningful(token: &str) -> bool {
    let t = token.trim();
    if t.is_empty() {
        return false;
    }
    // Drop single characters (mostly particles/punctuation noise).
    if t.chars().count() < 2 {
        return false;
    }
    // Drop pure numbers / punctuation / whitespace tokens.
    if t.chars().all(|c| !c.is_alphanumeric()) {
        return false;
    }
    if t.chars().all(|c| c.is_ascii_digit() || c.is_ascii_punctuation()) {
        return false;
    }
    if STOPWORDS.contains(t) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_stopwords_and_counts() {
        let msgs = vec![
            "我们今天一起去看电影吧".to_string(),
            "今天的电影真好看".to_string(),
            "电影结束后一起吃饭".to_string(),
        ];
        let out = top_words(&msgs, 10);
        let map: std::collections::HashMap<_, _> =
            out.iter().map(|w| (w.word.as_str(), w.count)).collect();

        // "电影" recurs across messages (jieba may fold "看电影" in one of them,
        // so assert it is a frequent term rather than an exact count).
        assert!(map.get("电影").copied().unwrap_or(0) >= 2);
        // Stopwords must be filtered out.
        assert!(!map.contains_key("我们"));
        assert!(!map.contains_key("的"));
    }

    #[test]
    fn top_n_is_sorted_desc_and_truncated() {
        let msgs = vec!["苹果 苹果 苹果 香蕉 香蕉 橙子".to_string()];
        let out = top_words(&msgs, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].word, "苹果");
        assert_eq!(out[0].count, 3);
        assert_eq!(out[1].word, "香蕉");
        assert!(out[0].count >= out[1].count);
    }

    #[test]
    fn drops_urls_and_numbers() {
        let msgs = vec!["看这个链接 https://example.com/abc 123456".to_string()];
        let out = top_words(&msgs, 20);
        assert!(out.iter().all(|w| !w.word.contains("http")));
        assert!(out.iter().all(|w| w.word.parse::<u64>().is_err()));
    }
}
