//! Subscribing to a feed. FreshRSS holds the subscriptions, and only one
//! side writes to it: this device in standalone mode, the server in server
//! mode. So a device paired with a server asks the server, there and then:
//! nothing is queued, and with the server out of reach the answer is
//! `Offline`.

use eddy_source_greader::GReaderError;
use eddy_sync::{AddFeedRequest, SyncError};

use crate::engine::Engine;
use crate::{EngineError, Result};

/// A subscription just made.
#[derive(Clone, Debug, PartialEq)]
pub struct AddedFeed {
    pub source_id: String,
    pub title: String,
    /// The folder it was filed in, if any.
    pub folder: Option<String>,
}

impl Engine {
    /// Subscribes to the feed at `url` (or the one the page there names),
    /// in `folder` if one is given: a folder of that name is made when
    /// there is none. The feed is listed when this returns; its articles
    /// come with the next refresh.
    pub async fn add_feed(&self, url: &str, folder: Option<&str>) -> Result<AddedFeed> {
        let url = feed_address(url)?;
        let folder = folder.map(str::trim).filter(|f| !f.is_empty());
        let inner = &self.inner;
        if inner.is_hub() && !inner.options.server_role {
            let client = inner.hub_client().ok_or(EngineError::NotConfigured)?;
            let result = client
                .add_feed(&AddFeedRequest {
                    url,
                    folder: folder.map(str::to_string),
                })
                .await
                .map_err(|e| match e {
                    // A server from before it could add feeds.
                    SyncError::Http(404, _) => EngineError::UpgradeRequired(
                        "服务端版本太旧，不能添加订阅源，请先更新 eddy-server。".into(),
                    ),
                    // FreshRSS would not subscribe; the server passes on why.
                    SyncError::Http(422, why) => EngineError::Invalid(why),
                    other => other.into(),
                });
            inner.note_hub(&result);
            let added = result?;
            return Ok(AddedFeed {
                source_id: added.source_id,
                title: added.title,
                folder: added.folder,
            });
        }
        let direct = inner
            .direct
            .read()
            .clone()
            .ok_or(EngineError::NotConfigured)?;
        // Not in the middle of a refresh, which drops the feeds FreshRSS
        // did not list when it began.
        let _guard = inner.refresh_lock.lock().await;
        direct
            .add_feed(inner, &url, folder)
            .await
            .map_err(|e| match e {
                // Not a failure to reach FreshRSS: its answer about this feed.
                EngineError::Upstream(GReaderError::Refused(_)) => {
                    EngineError::Invalid(e.to_string())
                }
                other => other,
            })
    }
}

impl Engine {
    /// Unsubscribes from a feed. It leaves the lists with its articles,
    /// starred ones included: FreshRSS deletes them with the feed. Pages
    /// saved from its articles stay. As with adding, a device paired with a
    /// server asks the server, and is told `Offline` when it is away.
    pub async fn remove_feed(&self, source_id: &str) -> Result<()> {
        let inner = &self.inner;
        let source = inner
            .store
            .source(source_id)?
            .filter(|s| s.kind == eddy_model::SourceKind::Feed)
            .ok_or(EngineError::NotFound)?;
        if inner.is_hub() && !inner.options.server_role {
            let client = inner.hub_client().ok_or(EngineError::NotConfigured)?;
            let result = client.remove_feed(source_id).await.map_err(|e| match e {
                SyncError::Http(404 | 405, _) => EngineError::UpgradeRequired(
                    "服务端版本太旧，不能取消订阅，请先更新 eddy-server。".into(),
                ),
                SyncError::Http(422, why) => EngineError::Invalid(why),
                other => other.into(),
            });
            inner.note_hub(&result);
            result?;
            // The server's word arrives as an op; no need to wait for it.
            inner.store.remove_source(source_id)?;
        } else {
            let direct = inner
                .direct
                .read()
                .clone()
                .ok_or(EngineError::NotConfigured)?;
            let _guard = inner.refresh_lock.lock().await;
            direct.remove_feed(inner, &source).await?;
        }
        // What only its articles used is not kept for a cache to trim.
        if let Err(e) = inner.store.purge_orphan_blobs() {
            inner
                .platform
                .log(crate::LogLevel::Warn, &format!("purge failed: {e}"));
        }
        Ok(())
    }
}

/// The address as it is sent on: http or https, nothing else.
fn feed_address(input: &str) -> Result<String> {
    let trimmed = input.trim();
    // `feed://example.com/rss` and `feed:https://…` are how some sites and
    // browsers write a feed's address.
    let trimmed = trimmed
        .strip_prefix("feed://")
        .map(|rest| format!("https://{rest}"))
        .or_else(|| trimmed.strip_prefix("feed:").map(str::to_string))
        .unwrap_or_else(|| trimmed.to_string());
    let with_scheme = if trimmed.contains("://") {
        trimmed
    } else {
        format!("https://{trimmed}")
    };
    match url::Url::parse(&with_scheme) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() => {
            Ok(url.into())
        }
        _ => Err(EngineError::Invalid(
            "请输入订阅源或网站的地址，例如 https://example.com/feed。".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_addresses_are_web_addresses() {
        for (given, sent) in [
            (
                " https://example.com/feed.xml ",
                "https://example.com/feed.xml",
            ),
            ("example.com/rss", "https://example.com/rss"),
            ("feed://example.com/rss", "https://example.com/rss"),
            ("feed:http://example.com/rss", "http://example.com/rss"),
        ] {
            assert_eq!(feed_address(given).unwrap(), sent, "{given}");
        }
        for bad in [
            "",
            "   ",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ftp://example.com/a",
        ] {
            assert!(feed_address(bad).is_err(), "{bad}");
        }
    }
}
