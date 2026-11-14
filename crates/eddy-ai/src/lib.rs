//! AI features built on the reader: paragraph translation, summaries and
//! tagging with the user's vocabulary. Providers support OpenAI-compatible
//! Chat Completions (`/v1`) and Gemini native (`/v1beta`) endpoints.

mod provider;
mod redact;
mod settings;
mod tasks;

pub use provider::{
    AiProtocol, ChatMessage, ChatProvider, OpenAiCompatible, Provider, ResolvedAiProtocol,
    completion_text, normalize_base, normalize_gemini_base, normalize_provider_url,
    parse_completion, parse_gemini_completion, resolve_protocol,
};
pub use redact::{model_failure, safe_detail};
pub use settings::{
    AiSettings, AutoAction, AutoRule, MAX_CONCURRENCY, ProviderSettings, RuleScope, lang_code_for,
};
pub use tasks::{
    PROMPT_VERSION, Summary, TagResult, batches, summarize, tag, translate_batch,
    validate_translations,
};

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Provider(String),
    #[error("{0}")]
    Format(String),
}

#[cfg(test)]
mod tests;
