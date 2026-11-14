use serde::{Deserialize, Deserializer, Serialize};

use crate::provider::AiProtocol;

/// Non-secret AI configuration. It syncs between devices as a setting; API
/// keys stay in each device's keychain under role-specific names.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AiSettings {
    pub translation_provider: Option<ProviderSettings>,
    /// Target language for translations, e.g. "简体中文".
    pub target_language: String,
    pub glossary: String,
    pub sync_translations: bool,
    /// Requests per day that automatic translation rules may spend.
    pub translation_daily_limit: u32,
    pub summary_provider: Option<ProviderSettings>,
    /// Output language for summaries, e.g. "简体中文".
    pub summary_language: String,
    pub sync_summaries: bool,
    /// Requests per day that automatic summary and tagging rules may spend.
    pub summary_daily_limit: u32,
    pub rules: Vec<AutoRule>,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            translation_provider: None,
            target_language: "简体中文".into(),
            glossary: String::new(),
            sync_translations: true,
            translation_daily_limit: 100,
            summary_provider: None,
            summary_language: "简体中文".into(),
            sync_summaries: true,
            summary_daily_limit: 100,
            rules: Vec::new(),
        }
    }
}

impl<'de> Deserialize<'de> for AiSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let map = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;
        let defaults = AiSettings::default();

        let legacy_provider: Option<ProviderSettings> = match map.get("provider") {
            Some(v) => serde_json::from_value(v.clone()).map_err(serde::de::Error::custom)?,
            None => None,
        };

        let translation_provider = if let Some(v) = map.get("translation_provider") {
            serde_json::from_value(v.clone()).map_err(serde::de::Error::custom)?
        } else {
            legacy_provider.clone()
        };

        let summary_provider = if let Some(v) = map.get("summary_provider") {
            serde_json::from_value(v.clone()).map_err(serde::de::Error::custom)?
        } else {
            legacy_provider
        };

        let target_language = map
            .get("target_language")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or(defaults.target_language);

        let summary_language = map
            .get("summary_language")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| target_language.clone());

        let glossary = map
            .get("glossary")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or(defaults.glossary);

        let sync_translations = map
            .get("sync_translations")
            .and_then(|v| v.as_bool())
            .unwrap_or(defaults.sync_translations);

        let sync_summaries = map
            .get("sync_summaries")
            .and_then(|v| v.as_bool())
            .unwrap_or(defaults.sync_summaries);

        let legacy_limit = map
            .get("daily_auto_limit")
            .and_then(|v| v.as_u64())
            .map(|v| v.min(u32::MAX as u64) as u32);

        let translation_daily_limit = map
            .get("translation_daily_limit")
            .and_then(|v| v.as_u64())
            .map(|v| v.min(u32::MAX as u64) as u32)
            .or(legacy_limit)
            .unwrap_or(defaults.translation_daily_limit);

        let summary_daily_limit = map
            .get("summary_daily_limit")
            .and_then(|v| v.as_u64())
            .map(|v| v.min(u32::MAX as u64) as u32)
            .or(legacy_limit)
            .unwrap_or(defaults.summary_daily_limit);

        let rules = match map.get("rules") {
            Some(v) => serde_json::from_value(v.clone()).map_err(serde::de::Error::custom)?,
            None => Vec::new(),
        };

        Ok(AiSettings {
            translation_provider,
            target_language,
            glossary,
            sync_translations,
            translation_daily_limit,
            summary_provider,
            summary_language,
            sync_summaries,
            summary_daily_limit,
            rules,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderSettings {
    #[serde(default)]
    pub protocol: AiProtocol,
    pub base_url: String,
    pub model: String,
    /// Requests sent to the service at once; some models and plans allow
    /// more than one.
    #[serde(default = "one")]
    pub max_concurrency: u32,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        ProviderSettings {
            protocol: AiProtocol::Auto,
            base_url: String::new(),
            model: String::new(),
            max_concurrency: 1,
        }
    }
}

/// Upper bound for `ProviderSettings::max_concurrency`.
pub const MAX_CONCURRENCY: u32 = 8;

fn one() -> u32 {
    1
}

impl ProviderSettings {
    pub fn concurrency(&self) -> usize {
        self.max_concurrency.clamp(1, MAX_CONCURRENCY) as usize
    }
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
    Some(
        if t.contains("中文") || t.contains("chinese") || t.starts_with("zh") {
            "cmn"
        } else if t.contains("english") || t.contains("英") || t == "en" {
            "eng"
        } else if t.contains("日本") || t.contains("japanese") || t == "ja" {
            "jpn"
        } else if t.contains("한국") || t.contains("korean") || t.contains("韩") || t == "ko" {
            "kor"
        } else if t.contains("français") || t.contains("french") || t.contains("法") {
            "fra"
        } else if t.contains("deutsch") || t.contains("german") || t.contains("德") || t == "de" {
            "deu"
        } else if t.contains("español") || t.contains("spanish") || t.contains("西班牙") {
            "spa"
        } else {
            return None;
        },
    )
}
