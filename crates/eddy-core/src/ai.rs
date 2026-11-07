use std::collections::HashMap;
use std::sync::Arc;

use eddy_ai::{
    AiSettings, AutoAction, OpenAiCompatible, PROMPT_VERSION, Provider, RuleScope, Summary,
    batches, lang_code_for, normalize_base,
};
use eddy_doc::translation_segments;
use eddy_model::{Entity, fields, ids};

use crate::content::LoadedDocument;
use crate::engine::{Engine, Inner};
use crate::{EngineError, LogLevel, Result};

const AI_KEY: &str = "ai:api-key";
const SETTINGS_KEY: &str = "ai";
const AUTO_CAP: usize = 20;
const AUTO_CONCURRENCY: usize = 2;

pub type BatchCallback = Arc<dyn Fn(&HashMap<String, String>) + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub struct TranslationResult {
    pub content_hash: String,
    pub target_language: String,
    /// Segment ID (block ID, `#caption`, `#rXcY`) → translated text.
    pub segments: HashMap<String, String>,
    pub cached: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SummaryResult {
    pub summary: Summary,
    pub cached: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TagInfo {
    pub id: String,
    pub name: String,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemTagsResult {
    /// Tags attached to the item (confirmed by the user or applied by AI).
    pub tags: Vec<TagInfo>,
    /// New tag names proposed by the model, waiting for the user.
    pub suggestions: Vec<String>,
}

fn artifact_key(parts: &[&str]) -> String {
    format!(
        "ai{}",
        &ids::sha256_hex(parts.join("\u{1f}").as_bytes())[..40]
    )
}

fn today() -> String {
    // UTC day; good enough for a soft daily budget.
    let days = eddy_model::now_ms() / 86_400_000;
    format!("d{days}")
}

impl Inner {
    pub(crate) fn ai_settings(&self) -> Result<AiSettings> {
        Ok(self
            .store
            .setting(SETTINGS_KEY)?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    fn provider(&self, settings: &AiSettings) -> Result<Arc<dyn Provider>> {
        let p = settings
            .provider
            .as_ref()
            .ok_or_else(|| EngineError::Invalid("还没有设置模型服务。".into()))?;
        let key = self
            .platform
            .secret_get(AI_KEY)
            .ok_or_else(|| EngineError::Invalid("还没有填写模型 API Key。".into()))?;
        let provider = OpenAiCompatible::new(&p.base_url, &p.model, &key)
            .map_err(|e| EngineError::Invalid(e.to_string()))?;
        Ok(Arc::new(provider))
    }

    fn ai_document(&self, item_id: &str) -> Result<(LoadedDocument, String)> {
        let row = self.store.item(item_id)?.ok_or(EngineError::NotFound)?;
        let doc = self
            .current_document(item_id)?
            .ok_or_else(|| EngineError::Invalid("这篇内容还没有正文。".into()))?;
        Ok((doc, row.item.title))
    }

    pub(crate) async fn translate(
        &self,
        item_id: &str,
        force: bool,
        on_batch: Option<BatchCallback>,
    ) -> Result<TranslationResult> {
        let settings = self.ai_settings()?;
        let provider = self.provider(&settings)?;
        let (doc, title) = self.ai_document(item_id)?;
        let target = settings.target_language.clone();
        let identity = provider.cache_identity();
        let item_key = artifact_key(&[
            "translation",
            &doc.version.hash,
            &identity,
            &target,
            &settings.glossary,
            PROMPT_VERSION,
        ]);
        if !force
            && let Some(payload) = self.store.ai_artifact(&item_key)?
            && let Ok(segments) = serde_json::from_str::<HashMap<String, String>>(&payload)
        {
            if let Some(cb) = &on_batch {
                cb(&segments);
            }
            return Ok(TranslationResult {
                content_hash: doc.version.hash,
                target_language: target,
                segments,
                cached: true,
            });
        }
        let mut segments = translation_segments(&doc.document);
        segments.insert(
            0,
            eddy_doc::Segment {
                id: "title".into(),
                text: title,
            },
        );
        let mut all = HashMap::new();
        let mut all_cached = true;
        for batch in batches(&segments) {
            let batch_json = serde_json::to_string(&batch)?;
            let batch_key = artifact_key(&[
                "translation_batch",
                &batch_json,
                &identity,
                &target,
                &settings.glossary,
                PROMPT_VERSION,
            ]);
            let cached = if force {
                None
            } else {
                self.store
                    .ai_artifact(&batch_key)?
                    .and_then(|p| serde_json::from_str::<HashMap<String, String>>(&p).ok())
            };
            let result = match cached {
                Some(r) => r,
                None => {
                    all_cached = false;
                    self.store.add_ai_usage(&today(), 1)?;
                    let r = eddy_ai::translate_batch(
                        provider.as_ref(),
                        &batch,
                        &target,
                        &settings.glossary,
                    )
                    .await
                    .map_err(|e| EngineError::Invalid(e.to_string()))?;
                    self.store.put_ai_artifact(
                        &batch_key,
                        item_id,
                        "translation_batch",
                        &doc.version.hash,
                        Some(&target),
                        Some(provider.model()),
                        &serde_json::to_string(&r)?,
                        false,
                    )?;
                    r
                }
            };
            if let Some(cb) = &on_batch {
                cb(&result);
            }
            all.extend(result);
        }
        self.store.put_ai_artifact(
            &item_key,
            item_id,
            "translation",
            &doc.version.hash,
            Some(&target),
            Some(provider.model()),
            &serde_json::to_string(&all)?,
            settings.sync_translations,
        )?;
        Ok(TranslationResult {
            content_hash: doc.version.hash,
            target_language: target,
            segments: all,
            cached: all_cached,
        })
    }

    pub(crate) async fn summarize(&self, item_id: &str, force: bool) -> Result<SummaryResult> {
        let settings = self.ai_settings()?;
        let provider = self.provider(&settings)?;
        let (doc, title) = self.ai_document(item_id)?;
        let key = artifact_key(&[
            "summary",
            &doc.version.hash,
            &provider.cache_identity(),
            &settings.target_language,
            PROMPT_VERSION,
        ]);
        if !force
            && let Some(summary) = self
                .store
                .ai_artifact(&key)?
                .and_then(|p| serde_json::from_str::<Summary>(&p).ok())
        {
            return Ok(SummaryResult {
                summary,
                cached: true,
            });
        }
        self.store.add_ai_usage(&today(), 1)?;
        let summary = eddy_ai::summarize(
            provider.as_ref(),
            &title,
            &doc.document.plain_text(),
            &settings.target_language,
        )
        .await
        .map_err(|e| EngineError::Invalid(e.to_string()))?;
        self.store.put_ai_artifact(
            &key,
            item_id,
            "summary",
            &doc.version.hash,
            Some(&settings.target_language),
            Some(provider.model()),
            &serde_json::to_string(&summary)?,
            settings.sync_summaries,
        )?;
        Ok(SummaryResult {
            summary,
            cached: false,
        })
    }

    /// Tags the item from the user's vocabulary. Tags the user rejected for
    /// this item are never re-applied; new names become suggestions.
    pub(crate) async fn suggest_tags(&self, item_id: &str) -> Result<ItemTagsResult> {
        let settings = self.ai_settings()?;
        let provider = self.provider(&settings)?;
        let (doc, title) = self.ai_document(item_id)?;
        let vocabulary = self.store.tags()?;
        let names: Vec<String> = vocabulary.iter().map(|(_, n)| n.clone()).collect();
        self.store.add_ai_usage(&today(), 1)?;
        let result = eddy_ai::tag(
            provider.as_ref(),
            &title,
            &doc.document.plain_text(),
            &names,
        )
        .await
        .map_err(|e| EngineError::Invalid(e.to_string()))?;
        let states: HashMap<String, String> =
            self.store.item_tag_states(item_id)?.into_iter().collect();
        for name in &result.tags {
            let Some((tag_id, _)) = vocabulary.iter().find(|(_, n)| n == name) else {
                continue;
            };
            if states.get(tag_id).is_none_or(|s| s == "removed") {
                self.record(
                    Entity::ItemTag,
                    &format!("{item_id}|{tag_id}"),
                    fields::STATE,
                    "auto".into(),
                    false,
                )?;
            }
        }
        self.store
            .set_tag_suggestions(item_id, &result.suggestions)?;
        self.item_tags(item_id)
    }

    pub(crate) fn item_tags(&self, item_id: &str) -> Result<ItemTagsResult> {
        let states: HashMap<String, String> =
            self.store.item_tag_states(item_id)?.into_iter().collect();
        let tags = self
            .store
            .tag_counts()?
            .into_iter()
            .filter(|(id, _, _)| {
                states
                    .get(id)
                    .is_some_and(|s| s == "confirmed" || s == "auto")
            })
            .map(|(id, name, count)| TagInfo { id, name, count })
            .collect();
        Ok(ItemTagsResult {
            tags,
            suggestions: self.store.tag_suggestions(item_id)?,
        })
    }

    /// Runs automatic AI rules on new items in the background, within the
    /// daily budget. Results already present (e.g. synced from another
    /// device) are reused instead of paid for again.
    pub(crate) fn auto_ai(self: &Arc<Self>, item_ids: Vec<String>) {
        let Ok(settings) = self.ai_settings() else {
            return;
        };
        if settings.rules.is_empty()
            || settings.provider.is_none()
            || self.platform.secret_get(AI_KEY).is_none()
        {
            return;
        }
        let target_code = lang_code_for(&settings.target_language);
        let mut jobs = Vec::new();
        for id in item_ids {
            let Ok(Some(row)) = self.store.item(&id) else {
                continue;
            };
            let source = row
                .item
                .source_id
                .as_deref()
                .and_then(|s| self.store.source(s).ok().flatten());
            for rule in &settings.rules {
                let in_scope = match &rule.scope {
                    RuleScope::All => true,
                    RuleScope::Source { id } => row.item.source_id.as_deref() == Some(id.as_str()),
                    RuleScope::Category { name } => {
                        source.as_ref().and_then(|s| s.category.as_deref()) == Some(name.as_str())
                    }
                };
                let long_enough = row.item.text_len >= rule.min_chars;
                let foreign = !rule.only_foreign
                    || (row.item.lang.is_some() && row.item.lang.as_deref() != target_code);
                if in_scope && long_enough && foreign && !jobs.contains(&(id.clone(), rule.action))
                {
                    jobs.push((id.clone(), rule.action));
                }
            }
            if jobs.len() >= AUTO_CAP {
                break;
            }
        }
        if jobs.is_empty() {
            return;
        }
        let inner = self.clone();
        let limit = settings.daily_auto_limit;
        tokio::spawn(async move {
            use futures::StreamExt;
            futures::stream::iter(jobs)
                .for_each_concurrent(AUTO_CONCURRENCY, |(id, action)| {
                    let inner = inner.clone();
                    async move {
                        if inner.store.ai_usage_today(&today()).unwrap_or(u32::MAX) >= limit {
                            return;
                        }
                        let result = match action {
                            AutoAction::Translate => {
                                inner.translate(&id, false, None).await.map(|_| ())
                            }
                            AutoAction::Summarize => inner.summarize(&id, false).await.map(|_| ()),
                            AutoAction::Tag => inner.suggest_tags(&id).await.map(|_| ()),
                        };
                        if let Err(e) = result {
                            inner
                                .platform
                                .log(LogLevel::Info, &format!("auto AI skipped: {e}"));
                        }
                    }
                })
                .await;
        });
    }
}

impl Engine {
    pub fn ai_settings(&self) -> Result<AiSettings> {
        self.inner.ai_settings()
    }

    /// Saves the (non-secret) AI settings; they sync like other settings.
    pub fn set_ai_settings(&self, mut settings: AiSettings) -> Result<()> {
        if let Some(p) = settings.provider.as_mut() {
            p.base_url =
                normalize_base(&p.base_url).map_err(|e| EngineError::Invalid(e.to_string()))?;
            p.model = p.model.trim().to_string();
            if p.model.is_empty() {
                return Err(EngineError::Invalid("请填写模型 ID。".into()));
            }
        }
        self.inner.record(
            Entity::Setting,
            SETTINGS_KEY,
            fields::VALUE,
            serde_json::to_value(&settings)?,
            false,
        )
    }

    /// Stores the API key in this device's keychain only; `None` removes it.
    pub fn set_ai_key(&self, key: Option<&str>) -> Result<()> {
        match key.map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => {
                if !self.inner.platform.secret_set(AI_KEY, k) {
                    return Err(EngineError::Invalid("无法把 API Key 保存到钥匙串。".into()));
                }
            }
            None => self.inner.platform.secret_delete(AI_KEY),
        }
        Ok(())
    }

    pub fn has_ai_key(&self) -> bool {
        self.inner.platform.secret_get(AI_KEY).is_some()
    }

    pub fn ai_usage_today(&self) -> Result<u32> {
        Ok(self.inner.store.ai_usage_today(&today())?)
    }

    /// Translates the item's current content paragraph by paragraph.
    /// `on_batch` receives partial results as batches complete.
    pub async fn translate(
        &self,
        item_id: &str,
        force: bool,
        on_batch: Option<BatchCallback>,
    ) -> Result<TranslationResult> {
        self.inner.translate(item_id, force, on_batch).await
    }

    pub async fn summarize(&self, item_id: &str, force: bool) -> Result<SummaryResult> {
        self.inner.summarize(item_id, force).await
    }

    pub async fn suggest_tags(&self, item_id: &str) -> Result<ItemTagsResult> {
        self.inner.suggest_tags(item_id).await
    }

    pub fn tags(&self) -> Result<Vec<TagInfo>> {
        Ok(self
            .inner
            .store
            .tag_counts()?
            .into_iter()
            .map(|(id, name, count)| TagInfo { id, name, count })
            .collect())
    }

    pub fn create_tag(&self, name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 40 {
            return Err(EngineError::Invalid("标签名需要 1 到 40 个字。".into()));
        }
        let id = ids::tag_id(name);
        self.inner.record(
            Entity::Tag,
            &id,
            fields::CREATE,
            serde_json::json!({ "name": name }),
            false,
        )?;
        Ok(id)
    }

    pub fn rename_tag(&self, tag_id: &str, name: &str) -> Result<()> {
        self.inner.record(
            Entity::Tag,
            tag_id,
            fields::TITLE,
            name.trim().into(),
            false,
        )
    }

    pub fn delete_tag(&self, tag_id: &str) -> Result<()> {
        self.inner
            .record(Entity::Tag, tag_id, fields::DELETED, true.into(), false)
    }

    pub fn item_tags(&self, item_id: &str) -> Result<ItemTagsResult> {
        self.inner.item_tags(item_id)
    }

    /// Attaches (confirmed) or rejects a tag on an item. A rejected tag is
    /// not re-applied by automatic tagging.
    pub fn set_item_tag(&self, item_id: &str, tag_id: &str, on: bool) -> Result<()> {
        let state = if on { "confirmed" } else { "rejected" };
        self.inner.record(
            Entity::ItemTag,
            &format!("{item_id}|{tag_id}"),
            fields::STATE,
            state.into(),
            false,
        )
    }

    /// Adds a suggested tag to the vocabulary and attaches it to the item.
    pub fn accept_tag_suggestion(&self, item_id: &str, name: &str) -> Result<()> {
        let tag_id = self.create_tag(name)?;
        self.set_item_tag(item_id, &tag_id, true)?;
        let rest: Vec<String> = self
            .inner
            .store
            .tag_suggestions(item_id)?
            .into_iter()
            .filter(|s| s != name)
            .collect();
        self.inner.store.set_tag_suggestions(item_id, &rest)?;
        Ok(())
    }

    /// Latest stored translation for the item's current content, if any.
    pub fn cached_translation(&self, item_id: &str) -> Result<Option<TranslationResult>> {
        let Some(doc) = self.inner.current_document(item_id)? else {
            return Ok(None);
        };
        let Some((hash, lang, payload)) = self
            .inner
            .store
            .latest_ai_artifact(item_id, "translation")?
        else {
            return Ok(None);
        };
        if hash != doc.version.hash {
            return Ok(None);
        }
        let segments = serde_json::from_str(&payload)?;
        Ok(Some(TranslationResult {
            content_hash: hash,
            target_language: lang.unwrap_or_default(),
            segments,
            cached: true,
        }))
    }

    pub fn cached_summary(&self, item_id: &str) -> Result<Option<Summary>> {
        let Some(doc) = self.inner.current_document(item_id)? else {
            return Ok(None);
        };
        let Some((hash, _, payload)) = self.inner.store.latest_ai_artifact(item_id, "summary")?
        else {
            return Ok(None);
        };
        if hash != doc.version.hash {
            return Ok(None);
        }
        Ok(serde_json::from_str(&payload).ok())
    }
}
