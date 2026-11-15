use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use eddy_ai::{
    AiError, AiSettings, AutoAction, ChatMessage, ChatProvider, PROMPT_VERSION, Provider,
    ProviderSettings, RuleScope, Summary, batches, lang_code_for, normalize_provider_url,
};
use eddy_doc::{ConvertOptions, Segment, html_to_document, translation_segments};
use eddy_extract::extract;
use eddy_model::{Document, Entity, fields, ids};
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use tokio::sync::Semaphore;

use crate::content::LoadedDocument;
use crate::engine::{Engine, Inner};
use crate::{EngineError, LogLevel, Result};

const LEGACY_AI_KEY: &str = "ai:api-key";
const TRANSLATE_AI_KEY: &str = "ai:translate:api-key";
const SUMMARY_AI_KEY: &str = "ai:summarize:api-key";
const AI_KEYS_MIGRATED_META: &str = "ai_keys_migrated_v1";
const SETTINGS_KEY: &str = "ai";
const AUTO_CAP: usize = 20;
const AUTO_CONCURRENCY: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AiRole {
    Translation,
    Summary,
}

impl AiRole {
    pub const fn secret_key(self) -> &'static str {
        match self {
            AiRole::Translation => TRANSLATE_AI_KEY,
            AiRole::Summary => SUMMARY_AI_KEY,
        }
    }

    pub(crate) fn usage_day_key(self) -> String {
        let prefix = match self {
            AiRole::Translation => "translate",
            AiRole::Summary => "summary",
        };
        format!("{prefix}:{}", today())
    }
}

pub type BatchCallback = Arc<dyn Fn(&HashMap<String, String>) + Send + Sync>;

/// Whether `text` is already written in the target language, so translating
/// it would only echo it back. `target_code` is the ISO 639-3 code from
/// `lang_code_for`; an unknown target skips nothing.
///
/// For Chinese, Japanese and Korean the script decides: when at least 80% of
/// the letters are in the target's script the paragraph is left alone, which
/// works for a paragraph of any length. For other languages statistical
/// detection is only trusted on paragraphs of 40+ characters that it reports
/// as reliable; anything doubtful is translated as before.
pub(crate) fn in_target_language(text: &str, target_code: Option<&str>) -> bool {
    let Some(code) = target_code else {
        return false;
    };
    let script_check: Option<fn(char) -> bool> = match code {
        "cmn" => Some(is_han),
        "jpn" => Some(|c| is_kana(c) || is_han(c)),
        "kor" => Some(is_hangul),
        _ => None,
    };
    if let Some(in_script) = script_check {
        let (mut letters, mut matched) = (0usize, 0usize);
        for c in text.chars().filter(|c| c.is_alphabetic()) {
            letters += 1;
            if in_script(c) {
                matched += 1;
            }
        }
        return letters > 0 && matched * 5 >= letters * 4;
    }
    if text.chars().count() < 40 {
        return false;
    }
    whatlang::detect(text).is_some_and(|info| info.is_reliable() && info.lang().code() == code)
}

fn is_han(c: char) -> bool {
    matches!(c,
        '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{20000}'..='\u{2FA1F}')
}

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF66}'..='\u{FF9F}')
}

fn is_hangul(c: char) -> bool {
    matches!(c,
        '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{A960}'..='\u{A97F}' | '\u{AC00}'..='\u{D7FF}')
}

/// Splits off the segments already in the target language. They come back
/// as their own translation without a request, so every segment asked for
/// still gets an answer; the reader hides a translation equal to its
/// source.
fn needing_translation(
    segments: Vec<Segment>,
    target: &str,
) -> (Vec<Segment>, HashMap<String, String>) {
    let code = lang_code_for(target);
    let mut keep = Vec::with_capacity(segments.len());
    let mut skipped = HashMap::new();
    for s in segments {
        if in_target_language(&s.text, code) {
            skipped.insert(s.id, s.text);
        } else {
            keep.push(s);
        }
    }
    (keep, skipped)
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranslationResult {
    pub content_hash: String,
    pub target_language: String,
    /// Segment ID (block ID, `#caption`, `#rXcY`) → translated text.
    pub segments: HashMap<String, String>,
    pub cached: bool,
    /// False for a translation stopped part-way; translating again
    /// finishes it, reusing the batches already done.
    pub complete: bool,
}

/// Outcome of `Engine::test_translation_provider` / `Engine::test_summary_provider`.
#[derive(Clone, Debug, PartialEq)]
pub struct AiTestResult {
    /// Requests sent at once.
    pub total: u32,
    pub passed: u32,
    pub elapsed_ms: u64,
    /// The first failure, already redacted.
    pub error: Option<String>,
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

/// Waits for a slot in the role's concurrency limit before each request.
struct Limited {
    inner: ChatProvider,
    gate: Arc<Semaphore>,
}

#[async_trait]
impl Provider for Limited {
    fn cache_identity(&self) -> String {
        self.inner.cache_identity()
    }

    fn model(&self) -> &str {
        self.inner.model()
    }

    async fn complete(&self, messages: &[ChatMessage]) -> std::result::Result<String, AiError> {
        let _permit = self.gate.acquire().await;
        self.inner.complete(messages).await
    }
}

/// For each segment's batch, by the block the segment belongs to.
fn batch_of_blocks(batches: &[Vec<Segment>]) -> HashMap<&str, usize> {
    let mut out = HashMap::new();
    for (i, batch) in batches.iter().enumerate() {
        for s in batch {
            let block = s.id.split('#').next().unwrap_or(&s.id);
            out.entry(block).or_insert(i);
        }
    }
    out
}

/// The batch holding `block`, or the next block with text (an image
/// without a caption has none), else the last one before it.
fn focus_batch(doc: &Document, of_block: &HashMap<&str, usize>, block: &str) -> Option<usize> {
    if let Some(&i) = of_block.get(block) {
        return Some(i);
    }
    let pos = doc.blocks.iter().position(|b| b.id == block)?;
    doc.blocks[pos..]
        .iter()
        .find_map(|b| of_block.get(b.id.as_str()).copied())
        .or_else(|| {
            doc.blocks[..pos]
                .iter()
                .rev()
                .find_map(|b| of_block.get(b.id.as_str()).copied())
        })
}

/// Position in `pending` of the batch to request next: the focus batch,
/// then outward from it, the one after before the one before.
fn nearest(pending: &[usize], focus: usize) -> usize {
    pending
        .iter()
        .enumerate()
        .min_by_key(|&(_, &i)| (i.abs_diff(focus), i < focus))
        .map(|(pos, _)| pos)
        .unwrap_or(0)
}

impl Inner {
    /// One-time migration copying the legacy single `ai:api-key` into both
    /// `ai:translate:api-key` and `ai:summarize:api-key` and removing the
    /// legacy entry so deleting either key later stays deleted.
    pub(crate) fn migrate_ai_keys(&self) -> Result<()> {
        if self.store.meta_get(AI_KEYS_MIGRATED_META)?.is_some() {
            return Ok(());
        }
        if let Some(legacy) = self
            .platform
            .secret_get(LEGACY_AI_KEY)
            .filter(|k| !k.trim().is_empty())
        {
            if self.platform.secret_get(TRANSLATE_AI_KEY).is_none() {
                let _ = self.platform.secret_set(TRANSLATE_AI_KEY, &legacy);
            }
            if self.platform.secret_get(SUMMARY_AI_KEY).is_none() {
                let _ = self.platform.secret_set(SUMMARY_AI_KEY, &legacy);
            }
            self.platform.secret_delete(LEGACY_AI_KEY);
        }
        self.store.meta_set(AI_KEYS_MIGRATED_META, "1")?;
        Ok(())
    }

    pub(crate) fn ai_settings(&self) -> Result<AiSettings> {
        Ok(self
            .store
            .setting(SETTINGS_KEY)?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    pub(crate) fn has_role_key(&self, role: AiRole) -> bool {
        self.platform
            .secret_get(role.secret_key())
            .is_some_and(|k| !k.trim().is_empty())
    }

    pub(crate) fn role_usage_today(&self, role: AiRole) -> Result<u32> {
        Ok(self.store.ai_usage_today(&role.usage_day_key())?)
    }

    fn record_role_usage(&self, role: AiRole, count: u32) -> Result<()> {
        self.store.add_ai_usage(&role.usage_day_key(), count)?;
        Ok(())
    }

    fn provider_for(&self, settings: &AiSettings, role: AiRole) -> Result<Arc<dyn Provider>> {
        let (provider_cfg, missing_provider_msg, missing_key_msg) = match role {
            AiRole::Translation => (
                settings.translation_provider.as_ref(),
                "还没有在 Translation 设置里配置翻译模型服务。",
                "还没有在 Translation 设置里填写 API Key。",
            ),
            AiRole::Summary => (
                settings.summary_provider.as_ref(),
                "还没有在 Summaries 设置里配置摘要模型服务。",
                "还没有在 Summaries 设置里填写 API Key。",
            ),
        };
        let p = provider_cfg.ok_or_else(|| EngineError::Invalid(missing_provider_msg.into()))?;
        let key = self
            .platform
            .secret_get(role.secret_key())
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| EngineError::Invalid(missing_key_msg.into()))?;
        self.limited_provider(role, p, &key)
    }

    /// A provider whose requests share the role's concurrency limit.
    fn limited_provider(
        &self,
        role: AiRole,
        p: &ProviderSettings,
        key: &str,
    ) -> Result<Arc<dyn Provider>> {
        let provider = ChatProvider::new(p.protocol, &p.base_url, &p.model, key)
            .map_err(|e| EngineError::Invalid(e.to_string()))?;
        Ok(Arc::new(Limited {
            inner: provider,
            gate: self.role_gate(role, p.concurrency()),
        }))
    }

    /// The per-role semaphore, replaced when the configured size changes;
    /// requests already holding the old one finish under it.
    fn role_gate(&self, role: AiRole, size: usize) -> Arc<Semaphore> {
        let mutex = match role {
            AiRole::Translation => &self.translation_gate,
            AiRole::Summary => &self.summary_gate,
        };
        let mut gate = mutex.lock();
        if gate.0 != size {
            *gate = (size, Arc::new(Semaphore::new(size)));
        }
        gate.1.clone()
    }

    /// Held while the request for `key` is out, so the same summary or
    /// batch asked for twice (a rule and the reader) is paid for once.
    fn ai_request(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut requests = self.ai_requests.lock();
        requests.retain(|_, lock| Arc::strong_count(lock) > 1);
        requests.entry(key.to_string()).or_default().clone()
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
        let provider = self.provider_for(&settings, AiRole::Translation)?;
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
        // A fresh run starts uncancelled.
        self.translation_cancels.lock().remove(item_id);
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
                complete: true,
            });
        }
        let partial_key = artifact_key(&[
            "translation_partial",
            &doc.version.hash,
            &identity,
            &target,
            &settings.glossary,
            PROMPT_VERSION,
        ]);
        // What is done so far stays on this device until the whole
        // translation is, and comes back from `cached_translation`.
        let save_partial = |all: &HashMap<String, String>| -> Result<()> {
            if all.is_empty() {
                return Ok(());
            }
            self.store.put_ai_artifact(
                &partial_key,
                item_id,
                "translation_partial",
                &doc.version.hash,
                Some(&target),
                Some(provider.model()),
                &serde_json::to_string(all)?,
                false,
            )?;
            Ok(())
        };
        let mut segments = translation_segments(&doc.document);
        segments.insert(
            0,
            eddy_doc::Segment {
                id: "title".into(),
                text: title,
            },
        );
        // Paragraphs already in the target language never go to the model.
        let (segments, skipped) = needing_translation(segments, &target);
        // Batches stay in document order so their cache keys don't depend on
        // where the reader is; only the order they are requested in does.
        let batches = batches(&segments);
        let mut keys = Vec::with_capacity(batches.len());
        for batch in &batches {
            keys.push(artifact_key(&[
                "translation_batch",
                &serde_json::to_string(batch)?,
                &identity,
                &target,
                &settings.glossary,
                PROMPT_VERSION,
            ]));
        }
        let mut all = HashMap::new();
        if !skipped.is_empty() {
            if let Some(cb) = &on_batch {
                cb(&skipped);
            }
            all.extend(skipped);
        }
        let mut pending = Vec::new();
        for (i, key) in keys.iter().enumerate() {
            let cached = if force {
                None
            } else {
                self.store
                    .ai_artifact(key)?
                    .and_then(|p| serde_json::from_str::<HashMap<String, String>>(&p).ok())
            };
            match cached {
                Some(r) => {
                    if let Some(cb) = &on_batch {
                        cb(&r);
                    }
                    all.extend(r);
                }
                None => pending.push(i),
            }
        }
        let all_cached = pending.is_empty();
        let of_block = batch_of_blocks(&batches);
        let limit = settings
            .translation_provider
            .as_ref()
            .map_or(1, ProviderSettings::concurrency);
        let mut in_flight = FuturesUnordered::new();
        let mut stopped = self.translation_cancels.lock().remove(item_id);
        let mut failure = None;
        loop {
            while !stopped && failure.is_none() && in_flight.len() < limit && !pending.is_empty() {
                let focus = self
                    .translation_focus
                    .lock()
                    .get(item_id)
                    .and_then(|b| focus_batch(&doc.document, &of_block, b))
                    .unwrap_or(0);
                let i = pending.remove(nearest(&pending, focus));
                let (provider, batch, key, target, glossary, hash) = (
                    provider.as_ref(),
                    &batches[i],
                    &keys[i],
                    &target,
                    &settings.glossary,
                    &doc.version.hash,
                );
                in_flight.push(async move {
                    let request = self.ai_request(key);
                    let _held = request.lock().await;
                    async {
                        // Another run may have done it while this one waited.
                        if !force
                            && let Some(r) = self
                                .store
                                .ai_artifact(key)?
                                .and_then(|p| serde_json::from_str(&p).ok())
                        {
                            return Ok(r);
                        }
                        self.record_role_usage(AiRole::Translation, 1)?;
                        let r = eddy_ai::translate_batch(provider, batch, target, glossary)
                            .await
                            .map_err(|e| EngineError::Invalid(e.to_string()))?;
                        self.store.put_ai_artifact(
                            key,
                            item_id,
                            "translation_batch",
                            hash,
                            Some(target),
                            Some(provider.model()),
                            &serde_json::to_string(&r)?,
                            false,
                        )?;
                        Ok::<HashMap<String, String>, EngineError>(r)
                    }
                    .await
                });
            }
            let Some(result) = in_flight.next().await else {
                break;
            };
            match result {
                Ok(r) => {
                    if let Some(cb) = &on_batch {
                        cb(&r);
                    }
                    all.extend(r);
                }
                Err(e) => {
                    failure.get_or_insert(e);
                }
            }
            stopped |= self.translation_cancels.lock().remove(item_id);
        }
        drop(in_flight);
        if let Some(e) = failure {
            save_partial(&all)?;
            return Err(e);
        }
        if !pending.is_empty() {
            save_partial(&all)?;
            return Ok(TranslationResult {
                content_hash: doc.version.hash.clone(),
                target_language: target.clone(),
                segments: all,
                cached: false,
                complete: false,
            });
        }
        self.translation_focus.lock().remove(item_id);
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
            complete: true,
        })
    }

    /// Translates `segments` with the configured translation provider, as many
    /// requests at once as it allows, counting each against the day's
    /// translation usage. Stops when cancelled under `key`, returning what is
    /// done; a failed batch stops it with the error.
    pub(crate) async fn translate_text(
        &self,
        key: &str,
        segments: Vec<Segment>,
        on_batch: Option<BatchCallback>,
    ) -> Result<HashMap<String, String>> {
        let settings = self.ai_settings()?;
        let provider = self.provider_for(&settings, AiRole::Translation)?;
        let target = settings.target_language.clone();
        self.translation_cancels.lock().remove(key);
        let (segments, skipped) = needing_translation(segments, &target);
        let batches = batches(&segments);
        let limit = settings
            .translation_provider
            .as_ref()
            .map_or(1, ProviderSettings::concurrency);
        let mut pending: Vec<usize> = (0..batches.len()).collect();
        let mut in_flight = FuturesUnordered::new();
        let mut all = HashMap::new();
        if !skipped.is_empty() {
            if let Some(cb) = &on_batch {
                cb(&skipped);
            }
            all.extend(skipped);
        }
        let mut stopped = false;
        let mut failure = None;
        loop {
            while !stopped && failure.is_none() && in_flight.len() < limit && !pending.is_empty() {
                let i = pending.remove(0);
                self.record_role_usage(AiRole::Translation, 1)?;
                let (provider, batch, target, glossary) =
                    (provider.as_ref(), &batches[i], &target, &settings.glossary);
                in_flight.push(async move {
                    eddy_ai::translate_batch(provider, batch, target, glossary).await
                });
            }
            let Some(result) = in_flight.next().await else {
                break;
            };
            match result {
                Ok(r) => {
                    if let Some(cb) = &on_batch {
                        cb(&r);
                    }
                    all.extend(r);
                }
                Err(e) => {
                    failure.get_or_insert(e);
                }
            }
            stopped |= self.translation_cancels.lock().remove(key);
        }
        drop(in_flight);
        if let Some(e) = failure {
            return Err(EngineError::Invalid(e.to_string()));
        }
        Ok(all)
    }

    pub(crate) async fn summarize(&self, item_id: &str, force: bool) -> Result<SummaryResult> {
        let settings = self.ai_settings()?;
        let provider = self.provider_for(&settings, AiRole::Summary)?;
        let (doc, title) = self.ai_document(item_id)?;
        let lang = settings.summary_language.clone();
        let key = artifact_key(&[
            "summary",
            &doc.version.hash,
            &provider.cache_identity(),
            &lang,
            PROMPT_VERSION,
        ]);
        let request = self.ai_request(&key);
        let _held = request.lock().await;
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
        self.record_role_usage(AiRole::Summary, 1)?;
        let summary =
            eddy_ai::summarize(provider.as_ref(), &title, &doc.document.plain_text(), &lang)
                .await
                .map_err(|e| EngineError::Invalid(e.to_string()))?;
        self.store.put_ai_artifact(
            &key,
            item_id,
            "summary",
            &doc.version.hash,
            Some(&lang),
            Some(provider.model()),
            &serde_json::to_string(&summary)?,
            settings.sync_summaries,
        )?;
        Ok(SummaryResult {
            summary,
            cached: false,
        })
    }

    /// Summarizes a page that isn't stored (one a link opened) from its
    /// HTML, the article extracted as saving would. Nothing is kept.
    pub(crate) async fn summarize_page(&self, url: &str, html: &str) -> Result<Summary> {
        let settings = self.ai_settings()?;
        let provider = self.provider_for(&settings, AiRole::Summary)?;
        let rule = self.fetcher.read().rule_for(url).cloned();
        let extracted =
            extract(html, url, rule.as_ref()).map_err(|e| EngineError::Invalid(e.to_string()))?;
        let document = html_to_document(
            &extracted.content_html,
            &ConvertOptions {
                base_url: Some(url),
                title: extracted.title.as_deref(),
            },
        );
        self.record_role_usage(AiRole::Summary, 1)?;
        eddy_ai::summarize(
            provider.as_ref(),
            extracted.title.as_deref().unwrap_or_default(),
            &document.plain_text(),
            &settings.summary_language,
        )
        .await
        .map_err(|e| EngineError::Invalid(e.to_string()))
    }

    /// Tags the item from the user's vocabulary. Tags the user rejected for
    /// this item are never re-applied; new names become suggestions.
    pub(crate) async fn suggest_tags(&self, item_id: &str) -> Result<ItemTagsResult> {
        let settings = self.ai_settings()?;
        let provider = self.provider_for(&settings, AiRole::Summary)?;
        let (doc, title) = self.ai_document(item_id)?;
        let vocabulary = self.store.tags()?;
        let names: Vec<String> = vocabulary.iter().map(|(_, n)| n.clone()).collect();
        self.record_role_usage(AiRole::Summary, 1)?;
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
        // Marks the item tagged, for automatic rules, even when no tag fit.
        self.store.put_ai_artifact(
            &artifact_key(&["tags", &doc.version.hash, &provider.cache_identity()]),
            item_id,
            "tags",
            &doc.version.hash,
            None,
            Some(provider.model()),
            &serde_json::to_string(&result.tags)?,
            false,
        )?;
        self.item_tags(item_id)
    }

    /// Tagged before, by AI or by hand: automatic rules leave it alone.
    pub(crate) fn tagged(&self, item_id: &str) -> Result<bool> {
        Ok(self.store.latest_ai_artifact(item_id, "tags")?.is_some()
            || !self.store.item_tag_states(item_id)?.is_empty()
            || !self.store.tag_suggestions(item_id)?.is_empty())
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

    /// Runs automatic AI rules on new items in the background, within each
    /// role's daily budget. Results already present (e.g. synced from another
    /// device) are reused instead of paid for again.
    pub(crate) fn auto_ai(self: &Arc<Self>, item_ids: Vec<String>) {
        let Ok(settings) = self.ai_settings() else {
            return;
        };
        let jobs = self.auto_jobs(&settings, item_ids);
        self.run_auto_jobs(
            jobs,
            settings.translation_daily_limit,
            settings.summary_daily_limit,
        );
    }

    /// The automatic rules for an item being read, within each role's daily
    /// budget: the summary and translation they ask for, which the reader
    /// shows, while the rest (tags) run in the background.
    pub(crate) fn auto_ai_on_open(self: &Arc<Self>, item_id: &str) -> (bool, bool) {
        let Ok(settings) = self.ai_settings() else {
            return (false, false);
        };
        let jobs = self.auto_jobs(&settings, vec![item_id.to_string()]);
        let trans_within_budget = || {
            self.role_usage_today(AiRole::Translation)
                .unwrap_or(u32::MAX)
                < settings.translation_daily_limit
        };
        let sum_within_budget = || {
            self.role_usage_today(AiRole::Summary).unwrap_or(u32::MAX)
                < settings.summary_daily_limit
        };
        let asks = |action| jobs.iter().any(|(_, a)| *a == action);
        let shown = (
            asks(AutoAction::Summarize) && sum_within_budget(),
            asks(AutoAction::Translate) && trans_within_budget(),
        );
        let rest = jobs
            .into_iter()
            .filter(|(_, a)| !matches!(a, AutoAction::Summarize | AutoAction::Translate))
            .collect();
        self.run_auto_jobs(
            rest,
            settings.translation_daily_limit,
            settings.summary_daily_limit,
        );
        shown
    }

    /// The (item, action) pairs the rules ask for; each action requires its
    /// corresponding role provider and key to be configured.
    fn auto_jobs(&self, settings: &AiSettings, item_ids: Vec<String>) -> Vec<(String, AutoAction)> {
        let mut jobs = Vec::new();
        if settings.rules.is_empty() {
            return jobs;
        }
        let trans_ready =
            settings.translation_provider.is_some() && self.has_role_key(AiRole::Translation);
        let sum_ready = settings.summary_provider.is_some() && self.has_role_key(AiRole::Summary);
        if !trans_ready && !sum_ready {
            return jobs;
        }
        let target_code = lang_code_for(&settings.target_language);
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
                let role_ready = match rule.action {
                    AutoAction::Translate => trans_ready,
                    AutoAction::Summarize | AutoAction::Tag => sum_ready,
                };
                if !role_ready {
                    continue;
                }
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
                if in_scope
                    && long_enough
                    && foreign
                    && !jobs.contains(&(id.clone(), rule.action))
                    && !(rule.action == AutoAction::Tag && self.tagged(&id).unwrap_or(true))
                {
                    jobs.push((id.clone(), rule.action));
                }
            }
            if jobs.len() >= AUTO_CAP {
                break;
            }
        }
        jobs
    }

    fn run_auto_jobs(
        self: &Arc<Self>,
        jobs: Vec<(String, AutoAction)>,
        translation_limit: u32,
        summary_limit: u32,
    ) {
        if jobs.is_empty() {
            return;
        }
        let inner = self.clone();
        tokio::spawn(async move {
            use futures::StreamExt;
            futures::stream::iter(jobs)
                .for_each_concurrent(AUTO_CONCURRENCY, |(id, action)| {
                    let inner = inner.clone();
                    async move {
                        let (role, limit) = match action {
                            AutoAction::Translate => (AiRole::Translation, translation_limit),
                            AutoAction::Summarize | AutoAction::Tag => {
                                (AiRole::Summary, summary_limit)
                            }
                        };
                        if inner.role_usage_today(role).unwrap_or(u32::MAX) >= limit {
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

fn normalize_provider_in_place(p: &mut ProviderSettings) -> Result<()> {
    p.model = p.model.trim().to_string();
    if p.model.is_empty() {
        return Err(EngineError::Invalid("请填写模型 ID。".into()));
    }
    p.base_url = normalize_provider_url(p.protocol, &p.base_url, &p.model)
        .map_err(|e| EngineError::Invalid(e.to_string()))?;
    p.max_concurrency = p.concurrency() as u32;
    Ok(())
}

impl Engine {
    pub fn ai_settings(&self) -> Result<AiSettings> {
        self.inner.ai_settings()
    }

    /// Saves the (non-secret) AI settings; they sync like other settings.
    pub fn set_ai_settings(&self, mut settings: AiSettings) -> Result<()> {
        if let Some(p) = settings.translation_provider.as_mut() {
            normalize_provider_in_place(p)?;
        }
        if let Some(p) = settings.summary_provider.as_mut() {
            normalize_provider_in_place(p)?;
        }
        settings.target_language = settings.target_language.trim().to_string();
        if settings.target_language.is_empty() {
            settings.target_language = "English".into();
        }
        settings.summary_language = settings.summary_language.trim().to_string();
        if settings.summary_language.is_empty() {
            settings.summary_language = settings.target_language.clone();
        }
        self.inner.record(
            Entity::Setting,
            SETTINGS_KEY,
            fields::VALUE,
            serde_json::to_value(&settings)?,
            false,
        )
    }

    /// Stores a role's API key in this device's keychain only; `None` removes it.
    pub fn set_ai_role_key(&self, role: AiRole, key: Option<&str>) -> Result<()> {
        match key.map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => {
                if !self.inner.platform.secret_set(role.secret_key(), k) {
                    return Err(EngineError::Invalid("无法把 API Key 保存到钥匙串。".into()));
                }
            }
            None => self.inner.platform.secret_delete(role.secret_key()),
        }
        Ok(())
    }

    pub fn has_ai_role_key(&self, role: AiRole) -> bool {
        self.inner.has_role_key(role)
    }

    /// Convenience helper setting both Translation and Summary keys at once.
    pub fn set_ai_key(&self, key: Option<&str>) -> Result<()> {
        self.set_ai_role_key(AiRole::Translation, key)?;
        self.set_ai_role_key(AiRole::Summary, key)
    }

    pub fn has_ai_key(&self) -> bool {
        self.has_ai_role_key(AiRole::Translation) || self.has_ai_role_key(AiRole::Summary)
    }

    pub fn ai_role_usage_today(&self, role: AiRole) -> Result<u32> {
        self.inner.role_usage_today(role)
    }

    pub fn ai_usage_today(&self) -> Result<u32> {
        Ok(self.ai_role_usage_today(AiRole::Translation)?
            + self.ai_role_usage_today(AiRole::Summary)?)
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

    /// Translates text that isn't a stored document: the paragraphs of a
    /// web page shown as the site made it. Nothing is kept or synced; the
    /// page's text is the page's, and changes with it. Batches arrive in
    /// `on_batch` as they finish, in document order. `key` names the run
    /// for `cancel_translation`.
    pub async fn translate_text(
        &self,
        key: &str,
        segments: Vec<Segment>,
        on_batch: Option<BatchCallback>,
    ) -> Result<HashMap<String, String>> {
        self.inner.translate_text(key, segments, on_batch).await
    }

    pub async fn summarize(&self, item_id: &str, force: bool) -> Result<SummaryResult> {
        self.inner.summarize(item_id, force).await
    }

    pub async fn summarize_page(&self, url: &str, html: &str) -> Result<Summary> {
        self.inner.summarize_page(url, html).await
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

    /// Latest stored translation for the item's current content: the whole
    /// one, else what a stopped translation into the current target
    /// language got done.
    pub fn cached_translation(&self, item_id: &str) -> Result<Option<TranslationResult>> {
        let Some(doc) = self.inner.current_document(item_id)? else {
            return Ok(None);
        };
        let store = &self.inner.store;
        if let Some((hash, lang, payload)) = store.latest_ai_artifact(item_id, "translation")?
            && hash == doc.version.hash
        {
            return Ok(Some(TranslationResult {
                content_hash: hash,
                target_language: lang.unwrap_or_default(),
                segments: serde_json::from_str(&payload)?,
                cached: true,
                complete: true,
            }));
        }
        let target = self.inner.ai_settings()?.target_language;
        if let Some((hash, lang, payload)) =
            store.latest_ai_artifact(item_id, "translation_partial")?
            && hash == doc.version.hash
            && lang.as_deref() == Some(target.as_str())
        {
            return Ok(Some(TranslationResult {
                content_hash: hash,
                target_language: target,
                segments: serde_json::from_str(&payload)?,
                cached: true,
                complete: false,
            }));
        }
        Ok(None)
    }

    /// The block being read; a running translation of the item requests
    /// the batches nearest to it next.
    pub fn set_translation_focus(&self, item_id: &str, block_id: &str) {
        self.inner
            .translation_focus
            .lock()
            .insert(item_id.to_string(), block_id.to_string());
    }

    /// Sends the configured number of translation requests at once with
    /// these (possibly unsaved) provider settings; `key` falls back to the
    /// saved translation key. Nothing is cached or counted against the daily budget.
    pub async fn test_translation_provider(
        &self,
        provider: ProviderSettings,
        key: Option<&str>,
        target_language: &str,
    ) -> Result<AiTestResult> {
        let key = match key.map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => k.to_string(),
            None => self
                .inner
                .platform
                .secret_get(AiRole::Translation.secret_key())
                .filter(|k| !k.trim().is_empty())
                .ok_or_else(|| EngineError::Invalid("还没有填写翻译 API Key。".into()))?,
        };
        let service =
            ChatProvider::new(provider.protocol, &provider.base_url, &provider.model, &key)
                .map_err(|e| EngineError::Invalid(e.to_string()))?;
        let total = provider.concurrency();
        let target = if target_language.trim().is_empty() {
            "English"
        } else {
            target_language.trim()
        };
        let sample = [
            Segment {
                id: "p1".into(),
                text: "The library opens at nine in the morning.".into(),
            },
            Segment {
                id: "p2".into(),
                text: "Readers can borrow up to five books at a time.".into(),
            },
        ];
        let started = std::time::Instant::now();
        let results = futures::future::join_all(
            (0..total).map(|_| eddy_ai::translate_batch(&service, &sample, target, "")),
        )
        .await;
        let error = results
            .iter()
            .find_map(|r| r.as_ref().err())
            .map(|e| e.to_string());
        Ok(AiTestResult {
            total: total as u32,
            passed: results.iter().filter(|r| r.is_ok()).count() as u32,
            elapsed_ms: started.elapsed().as_millis() as u64,
            error,
        })
    }

    /// Sends the configured number of summary requests at once with these
    /// (possibly unsaved) provider settings; `key` falls back to the saved
    /// summary key. Nothing is cached or counted against the daily budget.
    pub async fn test_summary_provider(
        &self,
        provider: ProviderSettings,
        key: Option<&str>,
        summary_language: &str,
    ) -> Result<AiTestResult> {
        let key = match key.map(str::trim).filter(|k| !k.is_empty()) {
            Some(k) => k.to_string(),
            None => self
                .inner
                .platform
                .secret_get(AiRole::Summary.secret_key())
                .filter(|k| !k.trim().is_empty())
                .ok_or_else(|| EngineError::Invalid("还没有填写摘要 API Key。".into()))?,
        };
        let service =
            ChatProvider::new(provider.protocol, &provider.base_url, &provider.model, &key)
                .map_err(|e| EngineError::Invalid(e.to_string()))?;
        let total = provider.concurrency();
        let lang = if summary_language.trim().is_empty() {
            "English"
        } else {
            summary_language.trim()
        };
        let sample_title = "Library Hours and Borrowing Policy";
        let sample_body = "The municipal library opens at nine in the morning from Monday through Saturday. \
            Registered readers may borrow up to five books at a time for twenty-one days, and renewals can be \
            requested online unless another reader has placed a hold.";
        let started = std::time::Instant::now();
        let results = futures::future::join_all(
            (0..total).map(|_| eddy_ai::summarize(&service, sample_title, sample_body, lang)),
        )
        .await;
        let error = results
            .iter()
            .find_map(|r| r.as_ref().err())
            .map(|e| e.to_string());
        Ok(AiTestResult {
            total: total as u32,
            passed: results.iter().filter(|r| r.is_ok()).count() as u32,
            elapsed_ms: started.elapsed().as_millis() as u64,
            error,
        })
    }

    pub async fn test_ai_provider(
        &self,
        provider: ProviderSettings,
        key: Option<&str>,
        target_language: &str,
    ) -> Result<AiTestResult> {
        self.test_translation_provider(provider, key, target_language)
            .await
    }

    /// Stops the item's translation once the batch in flight is done; what
    /// is done is kept.
    pub fn cancel_translation(&self, item_id: &str) {
        self.inner
            .translation_cancels
            .lock()
            .insert(item_id.to_string());
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
