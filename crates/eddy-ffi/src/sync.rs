use std::sync::Arc;

use crate::progress::{SearchResultView, TextRangeView};
use crate::{AccountView, EddyEngine, EddyError};

#[derive(uniffi::Record, Clone, Debug)]
pub struct ServerStatusView {
    pub url: String,
    pub server_id: String,
    pub account: Option<AccountView>,
    pub cursor: i64,
    pub video_capable: bool,
    /// The server's core version, e.g. `0.2.0 (1a2b3c4)`.
    pub version: String,
}

/// What signing out did.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SignOutView {
    /// Signed out; the data is erased on the next open.
    pub erased: bool,
    /// Not signed out: changes that exist only here and could not be sent.
    pub unsent: u32,
}

/// Hears about syncs that run in the background.
#[uniffi::export(with_foreign)]
pub trait SyncListener: Send + Sync {
    /// Local data changed, so lists can reload.
    fn on_change(&self);
    /// A long sync started, moved or ended; `status().sync` has the details.
    fn on_progress(&self);
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
            version: s.version,
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
    pub fn set_sync_listener(&self, listener: Option<Arc<dyn SyncListener>>) {
        let progress = listener.clone();
        self.engine.set_change_listener(
            listener.map(|l| Arc::new(move || l.on_change()) as Arc<dyn Fn() + Send + Sync>),
        );
        self.engine.set_progress_listener(
            progress.map(|l| Arc::new(move || l.on_progress()) as Arc<dyn Fn() + Send + Sync>),
        );
    }

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
    /// Pairs with eddy-server and returns as soon as the device is
    /// connected. The first sync (uploading local data, downloading the
    /// server's) carries on in the background and reports through the sync
    /// listener.
    pub async fn connect_server(
        &self,
        url: String,
        pairing_code: String,
        device_name: String,
    ) -> R<ServerStatusView> {
        let status = self
            .engine
            .pair_server(&url, &pairing_code, &device_name)
            .await?;
        self.engine.start_sync();
        Ok(status.into())
    }

    /// Leaves server mode and keeps the data; the server is asked to
    /// forget this device.
    pub async fn disconnect_server(&self) -> R<()> {
        Ok(self.engine.disconnect_server().await?)
    }

    /// Signs this device out and erases everything Eddy keeps on it. The
    /// files go when the data directory is next opened, so after `erased`
    /// the app restarts (or drops this engine and opens a new one).
    ///
    /// Changes not sent yet are sent first. When some cannot be, nothing is
    /// done and `unsent` says how many, unless `discard_unsent` is set.
    pub async fn sign_out_and_erase(&self, discard_unsent: bool) -> R<SignOutView> {
        Ok(
            match self.engine.sign_out_and_erase(discard_unsent).await? {
                eddy_core::SignOut::Erased => SignOutView {
                    erased: true,
                    unsent: 0,
                },
                eddy_core::SignOut::Unsent(unsent) => SignOutView {
                    erased: false,
                    unsent,
                },
            },
        )
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
