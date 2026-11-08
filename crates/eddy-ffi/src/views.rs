//! FFI view types. They mirror the domain model in shapes convenient for
//! native UIs; conversions live next to each type.

use eddy_core::{EngineStatus, ItemRow, Mode, RefreshSummary, SourceInfo};
use eddy_model as m;

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineMode {
    Unconfigured,
    Standalone,
    Server,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct AccountView {
    pub base_url: String,
    pub username: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct StatusView {
    pub mode: EngineMode,
    pub account: Option<AccountView>,
    pub last_refresh: Option<i64>,
    pub last_error: Option<String>,
}

impl From<EngineStatus> for StatusView {
    fn from(s: EngineStatus) -> Self {
        StatusView {
            mode: match s.mode {
                Mode::Unconfigured => EngineMode::Unconfigured,
                Mode::Standalone => EngineMode::Standalone,
                Mode::Server => EngineMode::Server,
            },
            account: s.account.map(|a| AccountView {
                base_url: a.base_url,
                username: a.username,
            }),
            last_refresh: s.last_refresh,
            last_error: s.last_error,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct RefreshSummaryView {
    pub new_items: u32,
    pub state_changes: u32,
    pub sources: u32,
}

impl From<RefreshSummary> for RefreshSummaryView {
    fn from(s: RefreshSummary) -> Self {
        RefreshSummaryView {
            new_items: s.new_items,
            state_changes: s.state_changes,
            sources: s.sources,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullTextModeView {
    Auto,
    Always,
    Never,
}

impl From<m::FullTextMode> for FullTextModeView {
    fn from(v: m::FullTextMode) -> Self {
        match v {
            m::FullTextMode::Auto => FullTextModeView::Auto,
            m::FullTextMode::Always => FullTextModeView::Always,
            m::FullTextMode::Never => FullTextModeView::Never,
        }
    }
}

impl From<FullTextModeView> for m::FullTextMode {
    fn from(v: FullTextModeView) -> Self {
        match v {
            FullTextModeView::Auto => m::FullTextMode::Auto,
            FullTextModeView::Always => m::FullTextMode::Always,
            FullTextModeView::Never => m::FullTextMode::Never,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SourceView {
    pub id: String,
    pub saved: bool,
    pub title: String,
    pub category: Option<String>,
    pub site_url: Option<String>,
    pub unread: u32,
    /// The mode its new items get under the full-text rules.
    pub full_text: FullTextModeView,
    /// The mode of a rule for this source alone, if it has one.
    pub source_rule: Option<FullTextModeView>,
    pub truncated: bool,
    /// Marked for the Gallery on its own (its folder can also be).
    pub gallery: bool,
}

impl SourceView {
    pub fn new(s: SourceInfo, rules: &[m::FullTextRule]) -> Self {
        let source_rule = rules
            .iter()
            .find(|r| matches!(&r.scope, m::FullTextScope::Source { id } if *id == s.source.id))
            .map(|r| r.mode.into());
        SourceView {
            full_text: m::resolve_full_text(&s.source, rules).into(),
            source_rule,
            id: s.source.id,
            saved: s.source.kind == m::SourceKind::Saved,
            title: s.source.title,
            category: s.source.category,
            site_url: s.source.site_url,
            unread: s.unread,
            truncated: s.source.truncated,
            gallery: s.source.gallery,
        }
    }
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum FullTextScopeView {
    All,
    Category { name: String },
    Source { id: String },
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct FullTextRuleView {
    pub scope: FullTextScopeView,
    pub mode: FullTextModeView,
}

impl From<m::FullTextRule> for FullTextRuleView {
    fn from(r: m::FullTextRule) -> Self {
        FullTextRuleView {
            scope: match r.scope {
                m::FullTextScope::All => FullTextScopeView::All,
                m::FullTextScope::Category { name } => FullTextScopeView::Category { name },
                m::FullTextScope::Source { id } => FullTextScopeView::Source { id },
            },
            mode: r.mode.into(),
        }
    }
}

impl From<FullTextRuleView> for m::FullTextRule {
    fn from(r: FullTextRuleView) -> Self {
        m::FullTextRule {
            scope: match r.scope {
                FullTextScopeView::All => m::FullTextScope::All,
                FullTextScopeView::Category { name } => m::FullTextScope::Category { name },
                FullTextScopeView::Source { id } => m::FullTextScope::Source { id },
            },
            mode: r.mode.into(),
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKindView {
    Feed,
    Saved,
    Chapter,
}

/// How to present an item: the reflowed reader, or a PDF view.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentFormatView {
    Article,
    Pdf,
}

impl From<m::ContentFormat> for ContentFormatView {
    fn from(f: m::ContentFormat) -> Self {
        match f {
            m::ContentFormat::Article => ContentFormatView::Article,
            m::ContentFormat::Pdf => ContentFormatView::Pdf,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ItemView {
    pub id: String,
    pub kind: ItemKindView,
    pub format: ContentFormatView,
    pub title: String,
    pub url: Option<String>,
    pub author: Option<String>,
    pub source_id: Option<String>,
    pub source_title: Option<String>,
    pub published_at: i64,
    pub read: bool,
    pub starred: bool,
    pub progress: f64,
    pub finished: bool,
    pub excerpt: Option<String>,
    pub lang: Option<String>,
    pub text_len: u32,
    pub series_id: Option<String>,
    pub chapter_index: Option<u32>,
    pub last_read_at: Option<i64>,
}

impl From<ItemRow> for ItemView {
    fn from(r: ItemRow) -> Self {
        let i = r.item;
        ItemView {
            id: i.id,
            kind: match i.kind {
                m::ItemKind::Feed => ItemKindView::Feed,
                m::ItemKind::Saved => ItemKindView::Saved,
                m::ItemKind::Chapter => ItemKindView::Chapter,
            },
            format: r.format.into(),
            title: i.title,
            url: i.url,
            author: i.author,
            source_id: i.source_id,
            source_title: r.source_title,
            published_at: i.published_at,
            read: i.read,
            starred: i.starred,
            progress: i.progress,
            finished: i.finished,
            excerpt: i.excerpt,
            lang: i.lang,
            text_len: i.text_len,
            series_id: i.series_id,
            chapter_index: i.chapter_index,
            last_read_at: i.last_read_at,
        }
    }
}

#[derive(uniffi::Enum, Clone, Debug)]
pub enum ItemFilterView {
    All,
    Starred,
    Saved,
    ContinueReading,
    Source {
        id: String,
    },
    Category {
        name: String,
    },
    Series {
        id: String,
    },
    Tag {
        id: String,
    },
    /// Articles and saved pages whose title contains `text`.
    Title {
        text: String,
    },
}

impl From<ItemFilterView> for eddy_core::ItemFilter {
    fn from(f: ItemFilterView) -> Self {
        use eddy_core::ItemFilter as F;
        match f {
            ItemFilterView::All => F::All,
            ItemFilterView::Starred => F::Starred,
            ItemFilterView::Saved => F::Saved,
            ItemFilterView::ContinueReading => F::ContinueReading,
            ItemFilterView::Source { id } => F::Source(id),
            ItemFilterView::Category { name } => F::Category(name),
            ItemFilterView::Series { id } => F::Series(id),
            ItemFilterView::Tag { id } => F::Tag(id),
            ItemFilterView::Title { text } => F::Title(text),
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ItemQueryView {
    pub filter: ItemFilterView,
    pub unread_only: bool,
    /// Listed by `unread_only` even when read: the article being read.
    pub keep: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

impl From<ItemQueryView> for eddy_core::ItemQuery {
    fn from(q: ItemQueryView) -> Self {
        eddy_core::ItemQuery {
            filter: q.filter.into(),
            unread_only: q.unread_only,
            keep: q.keep,
            limit: q.limit,
            offset: q.offset,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct RunView {
    pub text: String,
    pub style: u16,
    pub link: Option<String>,
    pub math: bool,
}

impl From<m::Run> for RunView {
    fn from(r: m::Run) -> Self {
        RunView {
            text: r.text,
            style: r.style,
            link: r.link,
            math: r.math,
        }
    }
}

fn runs(v: Vec<m::Run>) -> Vec<RunView> {
    v.into_iter().map(Into::into).collect()
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenClassView {
    Keyword,
    String,
    Comment,
    Number,
    Constant,
    Function,
    Type,
    Variable,
    Operator,
    Punctuation,
    Tag,
    Attribute,
}

impl From<m::TokenClass> for TokenClassView {
    fn from(c: m::TokenClass) -> Self {
        use m::TokenClass as T;
        match c {
            T::Keyword => Self::Keyword,
            T::String => Self::String,
            T::Comment => Self::Comment,
            T::Number => Self::Number,
            T::Constant => Self::Constant,
            T::Function => Self::Function,
            T::Type => Self::Type,
            T::Variable => Self::Variable,
            T::Operator => Self::Operator,
            T::Punctuation => Self::Punctuation,
            T::Tag => Self::Tag,
            T::Attribute => Self::Attribute,
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct HighlightView {
    pub start: u32,
    pub end: u32,
    pub class: TokenClassView,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TableCellView {
    pub header: bool,
    pub colspan: u16,
    pub rowspan: u16,
    pub runs: Vec<RunView>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TableRowView {
    pub cells: Vec<TableCellView>,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKindView {
    Video,
    Audio,
    Embed,
}

#[derive(uniffi::Enum, Clone, Debug)]
pub enum BlockKindView {
    Heading {
        level: u8,
        runs: Vec<RunView>,
    },
    Paragraph {
        runs: Vec<RunView>,
    },
    ListItem {
        ordered: bool,
        depth: u8,
        index: u32,
        marker: bool,
        runs: Vec<RunView>,
    },
    Code {
        language: Option<String>,
        text: String,
        highlights: Vec<HighlightView>,
    },
    Math {
        tex: String,
    },
    Image {
        src: String,
        alt: String,
        width: Option<u32>,
        height: Option<u32>,
        caption: Vec<RunView>,
    },
    Table {
        rows: Vec<TableRowView>,
    },
    Media {
        media: MediaKindView,
        src: String,
        poster: Option<String>,
        title: Option<String>,
    },
    Rule,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct BlockView {
    pub id: String,
    pub quote_depth: u8,
    pub kind: BlockKindView,
}

impl From<m::Block> for BlockView {
    fn from(b: m::Block) -> Self {
        use m::BlockKind as K;
        let kind = match b.kind {
            K::Heading { level, runs: r } => BlockKindView::Heading {
                level,
                runs: runs(r),
            },
            K::Paragraph { runs: r } => BlockKindView::Paragraph { runs: runs(r) },
            K::ListItem {
                ordered,
                depth,
                index,
                marker,
                runs: r,
            } => BlockKindView::ListItem {
                ordered,
                depth,
                index,
                marker,
                runs: runs(r),
            },
            K::Code {
                language,
                text,
                highlights,
            } => BlockKindView::Code {
                language,
                text,
                highlights: highlights
                    .into_iter()
                    .map(|h| HighlightView {
                        start: h.start,
                        end: h.end,
                        class: h.class.into(),
                    })
                    .collect(),
            },
            K::Math { tex } => BlockKindView::Math { tex },
            K::Image {
                src,
                alt,
                width,
                height,
                caption,
            } => BlockKindView::Image {
                src,
                alt,
                width,
                height,
                caption: runs(caption),
            },
            K::Table { rows } => BlockKindView::Table {
                rows: rows
                    .into_iter()
                    .map(|row| TableRowView {
                        cells: row
                            .cells
                            .into_iter()
                            .map(|c| TableCellView {
                                header: c.header,
                                colspan: c.colspan,
                                rowspan: c.rowspan,
                                runs: runs(c.runs),
                            })
                            .collect(),
                    })
                    .collect(),
            },
            K::Media {
                media,
                src,
                poster,
                title,
            } => BlockKindView::Media {
                media: match media {
                    m::MediaKind::Video => MediaKindView::Video,
                    m::MediaKind::Audio => MediaKindView::Audio,
                    m::MediaKind::Embed => MediaKindView::Embed,
                },
                src,
                poster,
                title,
            },
            K::Rule => BlockKindView::Rule,
        };
        BlockView {
            id: b.id,
            quote_depth: b.quote_depth,
            kind,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentOriginView {
    Feed,
    Extracted,
    Captured,
    Imported,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct DocumentView {
    pub content_hash: String,
    pub origin: ContentOriginView,
    pub format: ContentFormatView,
    pub source_url: Option<String>,
    /// For a PDF, its text (empty when it has none).
    pub blocks: Vec<BlockView>,
}

impl From<eddy_core::LoadedDocument> for DocumentView {
    fn from(d: eddy_core::LoadedDocument) -> Self {
        DocumentView {
            content_hash: d.version.hash,
            origin: match d.version.origin {
                m::ContentOrigin::Feed => ContentOriginView::Feed,
                m::ContentOrigin::Extracted => ContentOriginView::Extracted,
                m::ContentOrigin::Captured => ContentOriginView::Captured,
                m::ContentOrigin::Imported => ContentOriginView::Imported,
            },
            format: m::ContentFormat::from_mime(Some(&d.version.raw_mime)).into(),
            source_url: d.version.source_url,
            blocks: d.document.blocks.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ImageData {
    pub bytes: Vec<u8>,
    pub mime: String,
}
