use eddy_core as core;

use crate::{EddyEngine, EddyError, ItemView};

type R<T> = Result<T, EddyError>;

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaTypeView {
    Image,
    Video,
    Embed,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaStatusView {
    Ready,
    NoMedia,
    NeedsSignIn,
    Failed,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct MediaItemView {
    pub kind: MediaTypeView,
    pub src: String,
    pub poster: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<f64>,
    pub has_audio: bool,
    pub provider: Option<String>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct MediaSetView {
    pub status: MediaStatusView,
    pub items: Vec<MediaItemView>,
    pub thumbnail: Option<String>,
}

impl From<core::MediaSet> for MediaSetView {
    fn from(s: core::MediaSet) -> Self {
        MediaSetView {
            status: match s.status {
                core::MediaStatus::Ready => MediaStatusView::Ready,
                core::MediaStatus::NoMedia => MediaStatusView::NoMedia,
                core::MediaStatus::NeedsSignIn => MediaStatusView::NeedsSignIn,
                core::MediaStatus::Failed => MediaStatusView::Failed,
            },
            items: s
                .items
                .into_iter()
                .map(|m| MediaItemView {
                    kind: match m.kind {
                        core::MediaType::Image => MediaTypeView::Image,
                        core::MediaType::Video => MediaTypeView::Video,
                        core::MediaType::Embed => MediaTypeView::Embed,
                    },
                    src: m.src,
                    poster: m.poster,
                    width: m.width,
                    height: m.height,
                    duration: m.duration,
                    has_audio: m.has_audio,
                    provider: m.provider,
                })
                .collect(),
            thumbnail: s.thumbnail,
        }
    }
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq, Hash)]
pub enum GalleryScopeView {
    All,
    Source { id: String },
    Folder { name: String },
    Saved,
}

impl From<GalleryScopeView> for core::GalleryScope {
    fn from(s: GalleryScopeView) -> Self {
        match s {
            GalleryScopeView::All => core::GalleryScope::All,
            GalleryScopeView::Source { id } => core::GalleryScope::Source(id),
            GalleryScopeView::Folder { name } => core::GalleryScope::Folder(name),
            GalleryScopeView::Saved => core::GalleryScope::Saved,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct GalleryEntryView {
    pub item: ItemView,
    /// Resolved media, when resolved before.
    pub media: Option<MediaSetView>,
    pub thumbnail: Option<String>,
}

impl From<core::GalleryEntry> for GalleryEntryView {
    fn from(e: core::GalleryEntry) -> Self {
        GalleryEntryView {
            item: e.row.into(),
            media: e.media.map(Into::into),
            thumbnail: e.thumbnail,
        }
    }
}

#[uniffi::export]
impl EddyEngine {
    pub fn gallery_items(
        &self,
        scope: GalleryScopeView,
        unread_only: bool,
        limit: u32,
        offset: u32,
    ) -> R<Vec<GalleryEntryView>> {
        Ok(self
            .engine
            .gallery_items(&scope.into(), unread_only, limit, offset)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub fn gallery_count(&self, scope: GalleryScopeView, unread_only: bool) -> R<u32> {
        Ok(self.engine.gallery_count(&scope.into(), unread_only)?)
    }

    pub fn set_source_gallery(&self, source_id: String, shown: bool) -> R<()> {
        Ok(self.engine.set_source_gallery(&source_id, shown)?)
    }

    pub fn gallery_folders(&self) -> R<Vec<String>> {
        Ok(self.engine.gallery_folders()?)
    }

    pub fn set_folder_gallery(&self, name: String, shown: bool) -> R<()> {
        Ok(self.engine.set_folder_gallery(&name, shown)?)
    }

    /// Whether the item belongs in the Gallery.
    pub fn in_gallery(&self, item_id: String) -> R<bool> {
        let row = self.engine.item(&item_id)?.ok_or(EddyError::NotFound)?;
        Ok(self.engine.in_gallery(&row)?)
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl EddyEngine {
    /// The item's media; `force` asks the site again (after signing in).
    pub async fn resolve_media(&self, item_id: String, force: bool) -> R<MediaSetView> {
        Ok(self.engine.resolve_media(&item_id, force).await?.into())
    }

    /// Starts resolving these items in the background and returns at once.
    pub async fn prefetch_media(&self, item_ids: Vec<String>) {
        self.engine.prefetch_media(item_ids);
    }
}
