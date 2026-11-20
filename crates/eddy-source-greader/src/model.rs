use serde::Deserialize;

const LONG_PREFIX: &str = "tag:google.com,2005:reader/item/";

/// Normalizes an item ID to its 16-digit hex form. Google Reader APIs return
/// either the long tag form or a signed decimal.
pub fn normalize_item_id(id: &str) -> Option<String> {
    let id = id.trim();
    if let Some(hex) = id.strip_prefix(LONG_PREFIX) {
        let value = u64::from_str_radix(hex, 16).ok()?;
        return Some(format!("{value:016x}"));
    }
    let value: i64 = id.parse().ok()?;
    Some(format!("{:016x}", value as u64))
}

pub fn long_item_id(hex: &str) -> String {
    format!("{LONG_PREFIX}{hex}")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Subscription {
    pub id: String,
    pub title: String,
    pub category: Option<String>,
    pub site_url: Option<String>,
    pub feed_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpstreamItem {
    /// 16-digit hex item ID.
    pub id: String,
    pub feed_id: String,
    pub feed_title: String,
    pub title: String,
    pub url: Option<String>,
    pub author: Option<String>,
    pub published_ms: i64,
    pub html: String,
    pub read: bool,
    pub starred: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StreamPage {
    pub items: Vec<UpstreamItem>,
    pub continuation: Option<String>,
}

/// A subscription FreshRSS just made.
#[derive(Clone, Debug, PartialEq)]
pub struct QuickAdded {
    /// `feed/…`, as subscriptions are listed.
    pub stream_id: String,
    pub title: String,
}

/// FreshRSS answers `subscription/quickadd` with the new stream, or with
/// `numResults: 0` and why not.
#[derive(Deserialize)]
pub(crate) struct RawQuickAdd {
    #[serde(default, rename = "numResults")]
    pub num_results: u32,
    #[serde(default, rename = "streamId")]
    pub stream_id: Option<String>,
    #[serde(default, rename = "streamName")]
    pub stream_name: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RawSubscriptions {
    pub subscriptions: Vec<RawSubscription>,
}

#[derive(Deserialize)]
pub(crate) struct RawSubscription {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub categories: Vec<RawCategory>,
    #[serde(default, rename = "htmlUrl")]
    pub html_url: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RawCategory {
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RawStream {
    pub items: Vec<RawItem>,
    #[serde(default)]
    pub continuation: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RawItem {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub published: Option<serde_json::Value>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub alternate: Vec<RawLink>,
    #[serde(default)]
    pub canonical: Vec<RawLink>,
    #[serde(default)]
    pub summary: Option<RawContent>,
    #[serde(default)]
    pub content: Option<RawContent>,
    #[serde(default)]
    pub origin: Option<RawOrigin>,
}

#[derive(Deserialize)]
pub(crate) struct RawLink {
    #[serde(default)]
    pub href: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RawContent {
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RawOrigin {
    #[serde(default, rename = "streamId")]
    pub stream_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RawItemIds {
    #[serde(default, rename = "itemRefs")]
    pub item_refs: Vec<RawItemRef>,
    #[serde(default)]
    pub continuation: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RawItemRef {
    pub id: String,
}

impl From<RawSubscription> for Subscription {
    fn from(s: RawSubscription) -> Self {
        Subscription {
            title: if s.title.trim().is_empty() {
                "未命名订阅".into()
            } else {
                s.title
            },
            category: s
                .categories
                .into_iter()
                .find_map(|c| c.label)
                .filter(|l| !l.is_empty()),
            site_url: s.html_url.filter(|u| !u.is_empty()),
            feed_url: s.url.filter(|u| !u.is_empty()),
            id: s.id,
        }
    }
}

impl RawItem {
    pub(crate) fn into_item(self) -> Option<UpstreamItem> {
        let id = normalize_item_id(&self.id)?;
        let has = |suffix: &str| self.categories.iter().any(|c| c.ends_with(suffix));
        let read = has("/state/com.google/read");
        let starred = has("/state/com.google/starred");
        let published_ms = match &self.published {
            Some(serde_json::Value::Number(n)) => n.as_i64().unwrap_or(0) * 1000,
            Some(serde_json::Value::String(s)) => s.parse::<i64>().unwrap_or(0) * 1000,
            _ => 0,
        };
        let url = self
            .alternate
            .into_iter()
            .chain(self.canonical)
            .find_map(|l| l.href)
            .filter(|h| !h.is_empty());
        let html = self
            .content
            .and_then(|c| c.content)
            .or_else(|| self.summary.and_then(|c| c.content))
            .unwrap_or_default();
        let origin = self.origin;
        Some(UpstreamItem {
            id,
            feed_id: origin
                .as_ref()
                .and_then(|o| o.stream_id.clone())
                .unwrap_or_default(),
            feed_title: origin
                .and_then(|o| o.title)
                .unwrap_or_else(|| "未命名订阅".into()),
            title: self
                .title
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "无标题".into()),
            url,
            author: self.author.filter(|a| !a.is_empty()),
            published_ms,
            html,
            read,
            starred,
        })
    }
}
