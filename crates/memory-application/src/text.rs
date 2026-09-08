//! Chinese-aware lexical normalization adapted from the Apache-2.0 `mem0-memLayer` project.

use std::collections::HashSet;

use jieba_rs::Jieba;
use unicode_normalization::UnicodeNormalization;

pub const CHINESE_TOKENIZER_VERSION: &str = "jieba-rs-0.7.4-search-v1";

pub struct ChineseTokenizer {
    jieba: Jieba,
    stop_words: HashSet<&'static str>,
}

impl Default for ChineseTokenizer {
    fn default() -> Self {
        Self {
            jieba: Jieba::new(),
            stop_words: [
                "的", "了", "和", "是", "我", "你", "他", "她", "它", "在", "有", "与", "及", "把",
                "被", "这", "那", "一个", "一下", "请", "用户", "应该", "什么", "怎么", "如何",
                "是否",
            ]
            .into_iter()
            .collect(),
        }
    }
}

impl ChineseTokenizer {
    pub fn tokenize(&self, text: &str) -> Vec<String> {
        let normalized = normalize_text(text);
        self.jieba
            .cut_for_search(&normalized, true)
            .into_iter()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .filter(|token| !self.stop_words.contains(token))
            .filter(|token| token.chars().any(char::is_alphanumeric))
            .map(str::to_owned)
            .collect()
    }
}

pub fn normalize_text(text: &str) -> String {
    text.nfkc()
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_chinese_for_search_and_drops_stop_words() {
        let tokens =
            ChineseTokenizer::default().tokenize("用户应该如何排查 PostgreSQL 连接池故障？");
        assert!(tokens.iter().any(|token| token == "postgresql"));
        assert!(tokens.iter().any(|token| token == "连接池"));
        assert!(tokens.iter().any(|token| token == "故障"));
        assert!(!tokens.iter().any(|token| token == "用户"));
        assert!(!tokens.iter().any(|token| token == "如何"));
    }

    #[test]
    fn normalization_is_nfkc_lowercase_and_whitespace_stable() {
        assert_eq!(
            normalize_text(" ＰｏｓｔｇｒｅＳＱＬ   连接 "),
            "postgresql 连接"
        );
    }
}
