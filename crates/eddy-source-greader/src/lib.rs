//! Google Reader API client (FreshRSS, Miniflux and compatible services),
//! shared by standalone clients and the sync server.

mod client;
mod history;
mod model;

pub use client::{Credentials, GReaderClient};
pub use history::{HistoryScope, history_path, history_scope, validate_continuation};
pub use model::{
    QuickAdded, StreamPage, Subscription, UpstreamItem, long_item_id, normalize_item_id,
};

use url::Url;

pub const READING_LIST: &str = "user/-/state/com.google/reading-list";
pub const STARRED: &str = "user/-/state/com.google/starred";
pub const READ: &str = "user/-/state/com.google/read";

#[derive(Debug, thiserror::Error)]
pub enum GReaderError {
    #[error("{0}")]
    Config(String),
    #[error("FreshRSS 登录失败（{0}），请检查 API 是否启用，以及用户名和 API 专用密码。")]
    Auth(u16),
    #[error("FreshRSS 请求失败（HTTP {0}），请稍后重试。")]
    Http(u16),
    #[error("服务返回了跳转（HTTP {0}），请填写跳转后的 API 地址。")]
    Redirect(u16),
    #[error("连接服务超时，请稍后重试或检查网络。")]
    Timeout,
    #[error("无法连接 FreshRSS：{0}")]
    Network(String),
    #[error("FreshRSS 返回的数据格式不正确：{0}")]
    Format(String),
    /// FreshRSS answered, and would not subscribe: not a feed, already
    /// subscribed. Its own words.
    #[error("FreshRSS 没有添加这个订阅源：{0}")]
    Refused(String),
}

pub type Result<T> = std::result::Result<T, GReaderError>;

/// Normalizes a FreshRSS address to its Google Reader API base, keeping
/// custom ports and deployment prefixes: `https://host/reader/` becomes
/// `https://host/reader/api/greader.php`. Credentials, queries and fragments
/// are rejected so secrets are never sent to an unexpected endpoint.
pub fn normalize_base(input: &str) -> Result<String> {
    let mut url = Url::parse(input.trim()).map_err(|_| {
        GReaderError::Config("请输入完整的 FreshRSS 地址，例如 https://rss.example.com。".into())
    })?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return Err(GReaderError::Config(
            "FreshRSS 地址需要以 https:// 或 http:// 开头。".into(),
        ));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(GReaderError::Config(
            "地址里不能包含账号、参数或锚点。".into(),
        ));
    }
    let path = url.path().trim_end_matches('/').to_string();
    let path = if path.ends_with("/api/greader.php") {
        path
    } else {
        format!("{path}/api/greader.php")
    };
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests;
