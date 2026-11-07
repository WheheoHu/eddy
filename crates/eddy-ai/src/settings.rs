use serde::{Deserialize, Serialize};

/// Non-secret AI configuration. It syncs between devices as a setting; the
/// API key stays in each device's keychain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub provider: Option<ProviderSettings>,
    /// Target language as shown to the model, e.g. "简体中文".
    pub target_language: String,
    pub glossary: String,
    pub sync_translations: bool,
    pub sync_summaries: bool,
    /// Requests per day that automatic rules may spend.
    pub daily_auto_limit: u32,
    pub rules: Vec<AutoRule>,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            provider: None,
            target_language: "简体中文".into(),
            glossary: String::new(),
            sync_translations: true,
            sync_summaries: true,
            daily_auto_limit: 100,
            rules: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderSettings {
    pub base_url: String,
    pub model: String,
}

/// Runs an AI action automatically on new items that match.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutoRule {
    pub scope: RuleScope,
    pub action: AutoAction,
    /// Only items with at least this many readable characters.
    #[serde(default)]
    pub min_chars: u32,
    /// Only items not already in the target language.
    #[serde(default)]
    pub only_foreign: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleScope {
    All,
    Source { id: String },
    Category { name: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoAction {
    Translate,
    Summarize,
    Tag,
}

/// ISO 639-3 code (as produced by language detection) for a target language
/// name, so `only_foreign` can compare them.
pub fn lang_code_for(target: &str) -> Option<&'static str> {
    let t = target.to_lowercase();
    Some(if t.contains("中文") || t.contains("chinese") || t.starts_with("zh") {
        "cmn"
    } else if t.contains("english") || t.contains("英") || t == "en" {
        "eng"
    } else if t.contains("日本") || t.contains("japanese") || t == "ja" {
        "jpn"
    } else if t.contains("한국") || t.contains("korean") || t.contains("韩") || t == "ko" {
        "kor"
    } else if t.contains("français") || t.contains("french") || t.contains("法") {
        "fra"
    } else if t.contains("deutsch") || t.contains("german") || t.contains("德") {
        "deu"
    } else if t.contains("español") || t.contains("spanish") || t.contains("西班牙") {
        "spa"
    } else {
        return None;
    })
}
