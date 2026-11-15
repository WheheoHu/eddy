use std::sync::LazyLock;

use regex::Regex;

pub const REDACTED: &str = "[已隐藏]";

static HEADER_KEY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)((?:x-goog-api-key|x-api-key|api[-_]?key|access[-_]?token|secret|signature|ossaccesskeyid)\s*[:=]\s*['"]?)([^\s"',;&]+)"#).unwrap()
});
static URL_QUERY_SECRET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)([?&](?:key|api_key|token|signature|access_token|ossaccesskeyid)=)([^&#\s"']+)"#,
    )
    .unwrap()
});
static AIZA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bAIza[0-9A-Za-z_-]{20,}\b").unwrap());

/// Redacts credentials, bearer tokens, API keys and signed query parameters
/// using `[已隐藏]`, matching Eddy workspace security conventions.
pub fn redact_text(value: &str, secrets: &[&str]) -> String {
    let filtered: Vec<&str> = secrets
        .iter()
        .copied()
        .filter(|s| s.trim().len() >= 4)
        .collect();
    let mut out = eddy_ai::safe_detail(value, &filtered);
    for s in &filtered {
        out = out.replace(s.trim(), REDACTED);
    }
    let out = HEADER_KEY_RE.replace_all(&out, format!("${{1}}{REDACTED}"));
    let out = URL_QUERY_SECRET_RE.replace_all(&out, format!("${{1}}{REDACTED}"));
    AIZA_RE.replace_all(&out, REDACTED).into_owned()
}
