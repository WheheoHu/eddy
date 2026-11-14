use std::collections::HashMap;
use std::sync::Arc;

use eddy_core as core;

use crate::{EddyEngine, EddyError};

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiProtocolView {
    Auto,
    OpenAi,
    Gemini,
}

impl From<AiProtocolView> for core::AiProtocol {
    fn from(p: AiProtocolView) -> Self {
        match p {
            AiProtocolView::Auto => core::AiProtocol::Auto,
            AiProtocolView::OpenAi => core::AiProtocol::OpenAi,
            AiProtocolView::Gemini => core::AiProtocol::Gemini,
        }
    }
}

impl From<core::AiProtocol> for AiProtocolView {
    fn from(p: core::AiProtocol) -> Self {
        match p {
            core::AiProtocol::Auto => AiProtocolView::Auto,
            core::AiProtocol::OpenAi => AiProtocolView::OpenAi,
            core::AiProtocol::Gemini => AiProtocolView::Gemini,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiRoleView {
    Translation,
    Summary,
}

impl From<AiRoleView> for core::AiRole {
    fn from(r: AiRoleView) -> Self {
        match r {
            AiRoleView::Translation => core::AiRole::Translation,
            AiRoleView::Summary => core::AiRole::Summary,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ProviderView {
    pub protocol: AiProtocolView,
    pub base_url: String,
    pub model: String,
    /// Requests sent at once, 1 to `max_ai_concurrency()`.
    pub max_concurrency: u32,
}

impl From<ProviderView> for core::ProviderSettings {
    fn from(p: ProviderView) -> Self {
        core::ProviderSettings {
            protocol: p.protocol.into(),
            base_url: p.base_url,
            model: p.model,
            max_concurrency: p.max_concurrency,
        }
    }
}

impl From<core::ProviderSettings> for ProviderView {
    fn from(p: core::ProviderSettings) -> Self {
        ProviderView {
            protocol: p.protocol.into(),
            base_url: p.base_url,
            model: p.model,
            max_concurrency: p.max_concurrency,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct AiTestView {
    pub total: u32,
    pub passed: u32,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

#[derive(uniffi::Enum, Clone, Debug)]
pub enum RuleScopeView {
    All,
    Source { id: String },
    Category { name: String },
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoActionView {
    Translate,
    Summarize,
    Tag,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct AutoRuleView {
    pub scope: RuleScopeView,
    pub action: AutoActionView,
    pub min_chars: u32,
    pub only_foreign: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct AiSettingsView {
    pub translation_provider: Option<ProviderView>,
    pub target_language: String,
    pub glossary: String,
    pub sync_translations: bool,
    pub translation_daily_limit: u32,
    pub summary_provider: Option<ProviderView>,
    pub summary_language: String,
    pub sync_summaries: bool,
    pub summary_daily_limit: u32,
    pub rules: Vec<AutoRuleView>,
}

impl From<core::AiSettings> for AiSettingsView {
    fn from(s: core::AiSettings) -> Self {
        AiSettingsView {
            translation_provider: s.translation_provider.map(Into::into),
            target_language: s.target_language,
            glossary: s.glossary,
            sync_translations: s.sync_translations,
            translation_daily_limit: s.translation_daily_limit,
            summary_provider: s.summary_provider.map(Into::into),
            summary_language: s.summary_language,
            sync_summaries: s.sync_summaries,
            summary_daily_limit: s.summary_daily_limit,
            rules: s
                .rules
                .into_iter()
                .map(|r| AutoRuleView {
                    scope: match r.scope {
                        core::RuleScope::All => RuleScopeView::All,
                        core::RuleScope::Source { id } => RuleScopeView::Source { id },
                        core::RuleScope::Category { name } => RuleScopeView::Category { name },
                    },
                    action: match r.action {
                        core::AutoAction::Translate => AutoActionView::Translate,
                        core::AutoAction::Summarize => AutoActionView::Summarize,
                        core::AutoAction::Tag => AutoActionView::Tag,
                    },
                    min_chars: r.min_chars,
                    only_foreign: r.only_foreign,
                })
                .collect(),
        }
    }
}

impl From<AiSettingsView> for core::AiSettings {
    fn from(s: AiSettingsView) -> Self {
        core::AiSettings {
            translation_provider: s.translation_provider.map(Into::into),
            target_language: s.target_language,
            glossary: s.glossary,
            sync_translations: s.sync_translations,
            translation_daily_limit: s.translation_daily_limit,
            summary_provider: s.summary_provider.map(Into::into),
            summary_language: s.summary_language,
            sync_summaries: s.sync_summaries,
            summary_daily_limit: s.summary_daily_limit,
            rules: s
                .rules
                .into_iter()
                .map(|r| core::AutoRule {
                    scope: match r.scope {
                        RuleScopeView::All => core::RuleScope::All,
                        RuleScopeView::Source { id } => core::RuleScope::Source { id },
                        RuleScopeView::Category { name } => core::RuleScope::Category { name },
                    },
                    action: match r.action {
                        AutoActionView::Translate => core::AutoAction::Translate,
                        AutoActionView::Summarize => core::AutoAction::Summarize,
                        AutoActionView::Tag => core::AutoAction::Tag,
                    },
                    min_chars: r.min_chars,
                    only_foreign: r.only_foreign,
                })
                .collect(),
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TranslationView {
    pub content_hash: String,
    pub target_language: String,
    /// Segment ID (block ID, `title`, `<block>#caption`, `<block>#rXcY`) → text.
    pub segments: HashMap<String, String>,
    pub cached: bool,
    /// False for a translation stopped part-way.
    pub complete: bool,
}

impl From<core::TranslationResult> for TranslationView {
    fn from(t: core::TranslationResult) -> Self {
        TranslationView {
            content_hash: t.content_hash,
            target_language: t.target_language,
            segments: t.segments,
            cached: t.cached,
            complete: t.complete,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SummaryView {
    pub tldr: String,
    pub points: Vec<String>,
    pub cached: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TagView {
    pub id: String,
    pub name: String,
    pub count: u32,
}

impl From<core::TagInfo> for TagView {
    fn from(t: core::TagInfo) -> Self {
        TagView {
            id: t.id,
            name: t.name,
            count: t.count,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ItemTagsView {
    pub tags: Vec<TagView>,
    pub suggestions: Vec<String>,
}

impl From<core::ItemTagsResult> for ItemTagsView {
    fn from(r: core::ItemTagsResult) -> Self {
        ItemTagsView {
            tags: r.tags.into_iter().map(Into::into).collect(),
            suggestions: r.suggestions,
        }
    }
}

/// Receives partial translation results as batches complete.
#[uniffi::export(with_foreign)]
pub trait TranslationListener: Send + Sync {
    fn on_batch(&self, segments: HashMap<String, String>);
}

/// A piece of text to translate, named by the caller.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SegmentView {
    pub id: String,
    pub text: String,
}

type R<T> = Result<T, EddyError>;

#[uniffi::export(async_runtime = "tokio")]
impl EddyEngine {
    pub fn ai_settings(&self) -> R<AiSettingsView> {
        Ok(self.engine.ai_settings()?.into())
    }

    pub fn set_ai_settings(&self, settings: AiSettingsView) -> R<()> {
        Ok(self.engine.set_ai_settings(settings.into())?)
    }

    /// Stores a role's API key in this device's keychain; `None` removes it.
    pub fn set_ai_role_key(&self, role: AiRoleView, key: Option<String>) -> R<()> {
        Ok(self.engine.set_ai_role_key(role.into(), key.as_deref())?)
    }

    pub fn has_ai_role_key(&self, role: AiRoleView) -> bool {
        self.engine.has_ai_role_key(role.into())
    }

    pub fn ai_role_usage_today(&self, role: AiRoleView) -> R<u32> {
        Ok(self.engine.ai_role_usage_today(role.into())?)
    }

    pub fn ai_usage_today(&self) -> R<u32> {
        Ok(self.engine.ai_usage_today()?)
    }

    pub async fn translate(
        &self,
        item_id: String,
        force: bool,
        listener: Option<Arc<dyn TranslationListener>>,
    ) -> R<TranslationView> {
        let callback = listener.map(|l| {
            Arc::new(move |s: &HashMap<String, String>| l.on_batch(s.clone()))
                as core::BatchCallback
        });
        Ok(self
            .engine
            .translate(&item_id, force, callback)
            .await?
            .into())
    }

    /// Translates text that isn't stored content (a web page's paragraphs);
    /// nothing is kept or synced. `key` names the run for
    /// `cancel_translation`, which returns what is done so far.
    pub async fn translate_text(
        &self,
        key: String,
        segments: Vec<SegmentView>,
        listener: Option<Arc<dyn TranslationListener>>,
    ) -> R<HashMap<String, String>> {
        let callback = listener.map(|l| {
            Arc::new(move |s: &HashMap<String, String>| l.on_batch(s.clone()))
                as core::BatchCallback
        });
        let segments = segments
            .into_iter()
            .map(|s| core::Segment {
                id: s.id,
                text: s.text,
            })
            .collect();
        Ok(self.engine.translate_text(&key, segments, callback).await?)
    }

    pub fn cached_translation(&self, item_id: String) -> R<Option<TranslationView>> {
        Ok(self.engine.cached_translation(&item_id)?.map(Into::into))
    }

    /// The block being read (its engine block ID); a running translation
    /// of the item continues from there.
    pub fn set_translation_focus(&self, item_id: String, block_id: String) {
        self.engine.set_translation_focus(&item_id, &block_id)
    }

    pub fn max_ai_concurrency(&self) -> u32 {
        core::MAX_CONCURRENCY
    }

    /// Sends `provider.max_concurrency` translation requests at once with
    /// these settings; an empty `key` uses the saved translation key.
    pub async fn test_translation_provider(
        &self,
        provider: ProviderView,
        key: Option<String>,
        target_language: String,
    ) -> R<AiTestView> {
        let r = self
            .engine
            .test_translation_provider(provider.into(), key.as_deref(), &target_language)
            .await?;
        Ok(AiTestView {
            total: r.total,
            passed: r.passed,
            elapsed_ms: r.elapsed_ms,
            error: r.error,
        })
    }

    /// Sends `provider.max_concurrency` summary requests at once with
    /// these settings; an empty `key` uses the saved summary key.
    pub async fn test_summary_provider(
        &self,
        provider: ProviderView,
        key: Option<String>,
        summary_language: String,
    ) -> R<AiTestView> {
        let r = self
            .engine
            .test_summary_provider(provider.into(), key.as_deref(), &summary_language)
            .await?;
        Ok(AiTestView {
            total: r.total,
            passed: r.passed,
            elapsed_ms: r.elapsed_ms,
            error: r.error,
        })
    }

    /// Stops the item's translation once the batch in flight is done; the
    /// pending `translate` call then returns what is done.
    pub fn cancel_translation(&self, item_id: String) {
        self.engine.cancel_translation(&item_id)
    }

    pub async fn summarize(&self, item_id: String, force: bool) -> R<SummaryView> {
        let r = self.engine.summarize(&item_id, force).await?;
        Ok(SummaryView {
            tldr: r.summary.tldr,
            points: r.summary.points,
            cached: r.cached,
        })
    }

    /// A page a link opened, summarized from its HTML; nothing is stored.
    pub async fn summarize_page(&self, url: String, html: String) -> R<SummaryView> {
        let s = self.engine.summarize_page(&url, &html).await?;
        Ok(SummaryView {
            tldr: s.tldr,
            points: s.points,
            cached: false,
        })
    }

    pub fn cached_summary(&self, item_id: String) -> R<Option<SummaryView>> {
        Ok(self.engine.cached_summary(&item_id)?.map(|s| SummaryView {
            tldr: s.tldr,
            points: s.points,
            cached: true,
        }))
    }

    pub async fn suggest_tags(&self, item_id: String) -> R<ItemTagsView> {
        Ok(self.engine.suggest_tags(&item_id).await?.into())
    }

    pub fn tags(&self) -> R<Vec<TagView>> {
        Ok(self.engine.tags()?.into_iter().map(Into::into).collect())
    }

    pub fn create_tag(&self, name: String) -> R<String> {
        Ok(self.engine.create_tag(&name)?)
    }

    pub fn rename_tag(&self, tag_id: String, name: String) -> R<()> {
        Ok(self.engine.rename_tag(&tag_id, &name)?)
    }

    pub fn delete_tag(&self, tag_id: String) -> R<()> {
        Ok(self.engine.delete_tag(&tag_id)?)
    }

    pub fn item_tags(&self, item_id: String) -> R<ItemTagsView> {
        Ok(self.engine.item_tags(&item_id)?.into())
    }

    pub fn set_item_tag(&self, item_id: String, tag_id: String, on: bool) -> R<()> {
        Ok(self.engine.set_item_tag(&item_id, &tag_id, on)?)
    }

    pub fn accept_tag_suggestion(&self, item_id: String, name: String) -> R<()> {
        Ok(self.engine.accept_tag_suggestion(&item_id, &name)?)
    }
}
