//! Generic page-fetching pipeline: a direct request first, then the
//! strategies a site rule lists (custom headers, the user's own cookies, an
//! archived copy, a platform WebView) until the result is accepted.
//!
//! This crate ships no site rules; see README.md for the rule pack format.

mod rules;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use url::Url;

pub use rules::{RulePack, SiteRule, parse_ftr, to_css};

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("网址无效：{0}")]
    BadUrl(String),
    #[error("网页请求失败：HTTP {0}")]
    Http(u16),
    #[error("网络错误：{0}")]
    Network(String),
    #[error("不是网页内容（{0}）")]
    NotHtml(String),
    #[error("规则包格式错误：{0}")]
    Rules(String),
    #[error("所有抓取方式都没有拿到可用内容")]
    Exhausted,
}

/// A fetched page.
#[derive(Clone, Debug)]
pub struct Fetched {
    pub final_url: String,
    pub html: String,
    /// Which strategy produced it.
    pub strategy: &'static str,
}

/// Renders a page in a platform WebView (JavaScript, the user's own
/// browsing session) and returns its HTML. Implemented by the host app.
#[async_trait]
pub trait WebRenderer: Send + Sync {
    async fn render(&self, url: &str) -> Option<String>;
}

pub struct Fetcher {
    http: reqwest::Client,
    rules: RulePack,
    webview: Option<Arc<dyn WebRenderer>>,
    archive_base: String,
}

const MAX_PAGE_BYTES: usize = 15 * 1024 * 1024;
const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

impl Fetcher {
    pub fn new(
        rules: RulePack,
        webview: Option<Arc<dyn WebRenderer>>,
    ) -> Result<Fetcher, FetchError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|e| FetchError::Network(e.to_string()))?;
        Ok(Fetcher {
            http,
            rules,
            webview,
            archive_base: "https://archive.org".into(),
        })
    }

    /// Points archive lookups at another host (tests).
    pub fn with_archive_base(mut self, base: impl Into<String>) -> Self {
        self.archive_base = base.into();
        self
    }

    pub fn rules(&self) -> &RulePack {
        &self.rules
    }

    pub fn rule_for(&self, url: &str) -> Option<&SiteRule> {
        let host = Url::parse(url).ok()?.host_str()?.to_string();
        self.rules.rule_for(&host)
    }

    /// Fetches `url`, trying further strategies until `accept` approves the
    /// page. Returns the best page seen when none is accepted.
    pub async fn fetch(
        &self,
        url: &str,
        accept: impl Fn(&Fetched) -> bool,
    ) -> Result<Fetched, FetchError> {
        let parsed = Url::parse(url).map_err(|_| FetchError::BadUrl(url.into()))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(FetchError::BadUrl(url.into()));
        }
        let rule = parsed
            .host_str()
            .and_then(|h| self.rules.rule_for(h))
            .cloned()
            .unwrap_or_default();
        let mut order: Vec<String> = vec!["direct".into()];
        order.extend(rule.strategies.iter().cloned());

        let mut best: Option<Fetched> = None;
        // Unrendered pages of client-side renderers: used only as a last resort.
        let mut unrendered: Option<Fetched> = None;
        let mut last_error = None;
        let mut i = 0;
        while i < order.len() {
            let strategy = order[i].clone();
            i += 1;
            let result = match strategy.as_str() {
                "direct" => self.get(&parsed, &HeaderMap::new(), "direct").await,
                "headers" => {
                    self.get(&parsed, &rule_headers(&rule, false), "headers")
                        .await
                }
                "cookies" => {
                    self.get(&parsed, &rule_headers(&rule, true), "cookies")
                        .await
                }
                "archive" => self.archive(&parsed).await,
                "webview" => self.webview(&parsed).await,
                _ => continue,
            };
            match result {
                Ok(page) => {
                    if page.strategy != "webview" && needs_js(&page.html) && self.webview.is_some()
                    {
                        if !order.iter().any(|s| s == "webview") {
                            order.push("webview".into());
                        }
                        unrendered.get_or_insert(page);
                        continue;
                    }
                    if accept(&page) {
                        return Ok(page);
                    }
                    if best.as_ref().is_none_or(|b| page.html.len() > b.html.len()) {
                        best = Some(page);
                    }
                }
                Err(e) => last_error = Some(e),
            }
        }
        best.or(unrendered)
            .ok_or(last_error.unwrap_or(FetchError::Exhausted))
    }

    /// Renders `url` in the platform WebView, if one is installed.
    pub async fn render(&self, url: &str) -> Option<Fetched> {
        let parsed = Url::parse(url).ok()?;
        self.webview(&parsed).await.ok()
    }

    pub fn has_webview(&self) -> bool {
        self.webview.is_some()
    }

    async fn get(
        &self,
        url: &Url,
        extra: &HeaderMap,
        strategy: &'static str,
    ) -> Result<Fetched, FetchError> {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, HeaderValue::from_static(BROWSER_UA));
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml;q=0.9,*/*;q=0.8"),
        );
        headers.insert(
            header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
        );
        for (k, v) in extra {
            headers.insert(k.clone(), v.clone());
        }
        let response = self
            .http
            .get(url.clone())
            .headers(headers)
            .send()
            .await
            .map_err(|e| FetchError::Network(short_error(&e)))?;
        let status = response.status();
        if !status.is_success() {
            return Err(FetchError::Http(status.as_u16()));
        }
        let final_url = response.url().to_string();
        let mime = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        if !mime.is_empty()
            && !mime.contains("html")
            && !mime.contains("xml")
            && !mime.starts_with("text/")
        {
            return Err(FetchError::NotHtml(mime));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| FetchError::Network(short_error(&e)))?;
        if bytes.len() > MAX_PAGE_BYTES {
            return Err(FetchError::NotHtml("页面过大".into()));
        }
        Ok(Fetched {
            final_url,
            html: decode_html(&bytes, &mime),
            strategy,
        })
    }

    async fn archive(&self, url: &Url) -> Result<Fetched, FetchError> {
        let api = format!("{}/wayback/available", self.archive_base);
        let response = self
            .http
            .get(&api)
            .query(&[("url", url.as_str())])
            .send()
            .await
            .map_err(|e| FetchError::Network(short_error(&e)))?;
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| FetchError::Network(short_error(&e)))?;
        let snapshot = &json["archived_snapshots"]["closest"];
        let (Some(true), Some(ts)) = (
            snapshot["available"].as_bool(),
            snapshot["timestamp"].as_str(),
        ) else {
            return Err(FetchError::Exhausted);
        };
        // `id_` asks for the original page without the Wayback toolbar.
        let raw = Url::parse(&format!(
            "{}/web/{ts}id_/{}",
            self.archive_base
                .replace("://archive.org", "://web.archive.org"),
            url
        ))
        .map_err(|_| FetchError::BadUrl(url.to_string()))?;
        let mut page = self.get(&raw, &HeaderMap::new(), "archive").await?;
        page.final_url = url.to_string();
        Ok(page)
    }

    async fn webview(&self, url: &Url) -> Result<Fetched, FetchError> {
        let renderer = self.webview.as_ref().ok_or(FetchError::Exhausted)?;
        let html = renderer
            .render(url.as_str())
            .await
            .ok_or(FetchError::Exhausted)?;
        Ok(Fetched {
            final_url: url.to_string(),
            html,
            strategy: "webview",
        })
    }
}

static SCRIPT_OR_STYLE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?is)<(script|style|noscript)\b.*?</(script|style|noscript)>").unwrap()
});
static TAG: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?s)<[^>]*>").unwrap());

/// Pages whose content only exists after JavaScript runs: client-side
/// document renderers (Markdeep, Docsify) and empty single-page-app shells.
pub fn needs_js(html: &str) -> bool {
    let lower_head: String = html
        .chars()
        .take(200_000)
        .collect::<String>()
        .to_lowercase();
    if lower_head.contains("markdeep.min.js") || lower_head.contains("window.$docsify") {
        return true;
    }
    let shell = lower_head.contains("<div id=\"root\"></div>")
        || lower_head.contains("<div id=\"app\"></div>")
        || lower_head.contains("enable javascript");
    if !shell {
        return false;
    }
    let visible = TAG
        .replace_all(&SCRIPT_OR_STYLE.replace_all(html, " "), " ")
        .chars()
        .filter(|c| !c.is_whitespace())
        .count();
    visible < 500
}

fn rule_headers(rule: &SiteRule, with_cookies: bool) -> HeaderMap {
    let mut h = HeaderMap::new();
    if let Some(v) = rule
        .user_agent
        .as_deref()
        .and_then(|v| HeaderValue::from_str(v).ok())
    {
        h.insert(header::USER_AGENT, v);
    }
    if let Some(v) = rule
        .referer
        .as_deref()
        .and_then(|v| HeaderValue::from_str(v).ok())
    {
        h.insert(header::REFERER, v);
    }
    for (k, v) in &rule.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            h.insert(name, value);
        }
    }
    if with_cookies && !rule.cookies.is_empty() {
        let cookie = rule
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        if let Ok(v) = HeaderValue::from_str(&cookie) {
            h.insert(header::COOKIE, v);
        }
    }
    h
}

/// Decodes page bytes using the charset from the Content-Type header or a
/// `<meta charset>`, defaulting to UTF-8.
fn decode_html(bytes: &[u8], mime: &str) -> String {
    let from_header = mime
        .split("charset=")
        .nth(1)
        .map(|c| c.trim_matches(['"', ' ', ';']).to_string());
    let from_meta = || {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_lowercase();
        let i = head.find("charset=")?;
        let rest = &head[i + 8..];
        let rest = rest.trim_start_matches(['"', '\'']);
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len());
        Some(rest[..end].to_string())
    };
    let charset = from_header
        .or_else(from_meta)
        .unwrap_or_else(|| "utf-8".into());
    match charset.as_str() {
        "gbk" | "gb2312" | "gb18030" | "big5" | "shift_jis" | "euc-kr" | "windows-1252"
        | "iso-8859-1" => {
            // reqwest's charset feature brings encoding_rs.
            let enc = reqwest_encoding(&charset);
            enc.map(|e| e.decode(bytes).0.into_owned())
                .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn reqwest_encoding(label: &str) -> Option<&'static encoding_rs::Encoding> {
    encoding_rs::Encoding::for_label(label.as_bytes())
}

fn short_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "请求超时".into();
    }
    let mut msg = e.to_string();
    if let Some(url) = e.url() {
        msg = msg.replace(url.as_str(), url.host_str().unwrap_or(""));
    }
    msg
}

#[cfg(test)]
mod tests;
