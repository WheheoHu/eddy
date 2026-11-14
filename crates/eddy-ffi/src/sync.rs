use crate::progress::{SearchResultView, TextRangeView};
use crate::{AccountView, EddyEngine, EddyError};

#[derive(uniffi::Record, Clone, Debug)]
pub struct ServerStatusView {
    pub url: String,
    pub server_id: String,
    pub account: Option<AccountView>,
    pub cursor: i64,
    pub video_capable: bool,
}

impl From<eddy_core::ServerStatus> for ServerStatusView {
    fn from(s: eddy_core::ServerStatus) -> Self {
        ServerStatusView {
            url: s.url,
            server_id: s.server_id,
            account: s.account.map(|a| AccountView {
                base_url: a.base_url,
                username: a.username,
            }),
            cursor: s.cursor,
            video_capable: s.video_capable,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SourceStatsView {
    pub source_id: String,
    pub total: u32,
    pub unread: u32,
    pub latest_published_at: Option<i64>,
    pub last_read_at: Option<i64>,
}

type R<T> = Result<T, EddyError>;

#[uniffi::export]
impl EddyEngine {
    /// Per-source totals (articles, unread, latest update, last read) for
    /// library shelves.
    pub fn source_overview(&self) -> R<Vec<SourceStatsView>> {
        Ok(self
            .engine
            .source_overview()?
            .into_iter()
            .map(|s| SourceStatsView {
                source_id: s.source_id,
                total: s.total,
                unread: s.unread,
                latest_published_at: s.latest_published_at,
                last_read_at: s.last_read_at,
            })
            .collect())
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl EddyEngine {
    /// Pairs with eddy-server and migrates local data to it.
    pub async fn connect_server(
        &self,
        url: String,
        pairing_code: String,
        device_name: String,
    ) -> R<ServerStatusView> {
        Ok(self
            .engine
            .connect_server(&url, &pairing_code, &device_name)
            .await?
            .into())
    }

    pub fn disconnect_server(&self) -> R<()> {
        Ok(self.engine.disconnect_server()?)
    }

    pub async fn server_status(&self) -> R<Option<ServerStatusView>> {
        Ok(self.engine.server_status().await?.map(Into::into))
    }

    pub async fn configure_server_freshrss(
        &self,
        url: String,
        username: String,
        password: String,
    ) -> R<()> {
        Ok(self
            .engine
            .configure_server_freshrss(&url, &username, &password)
            .await?)
    }

    /// Full-history search on the server, local search when offline.
    pub async fn search_all(&self, query: String, limit: u32) -> R<Vec<SearchResultView>> {
        Ok(self
            .engine
            .search_all(&query, limit)
            .await?
            .into_iter()
            .map(|r| SearchResultView {
                item: r.item.into(),
                snippet: r.snippet,
                highlights: r
                    .highlights
                    .into_iter()
                    .map(|(start, end)| TextRangeView { start, end })
                    .collect(),
            })
            .collect())
    }

    /// Listens for server changes while the app is in the foreground.
    pub async fn start_live_sync(&self) {
        self.engine.start_live_sync();
    }

    pub fn stop_live_sync(&self) {
        self.engine.stop_live_sync();
    }
}
