//! AI features built on the reader: paragraph translation, summaries and
//! tagging with the user's vocabulary. Providers are pluggable; the first is
//! any OpenAI-compatible Chat Completions endpoint.

mod provider;
mod redact;
mod settings;
mod tasks;

pub use provider::{
    ChatMessage, OpenAiCompatible, Provider, completion_text, normalize_base, parse_completion,
};
pub use redact::{model_failure, safe_detail};
pub use settings::{AiSettings, AutoAction, AutoRule, ProviderSettings, RuleScope, lang_code_for};
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
