//! What goes over the control socket: one request per connection, as a line
//! of JSON, answered by progress lines and then one line with the result or
//! the error.

use serde::{Deserialize, Serialize};

/// Raised when a request or a result changes in a way an older peer would
/// misread. The command line and the app it talks to come from the same
/// bundle, so a mismatch means the app has been running since before an
/// update.
pub const CONTROL_VERSION: u32 = 1;

/// A request with the version of the protocol it is written in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    /// The sender's core version, for the message when versions differ.
    pub core: String,
    pub request: Request,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", content = "args")]
pub enum Request {
    /// Answers with the versions on the other end; how a client finds out
    /// that the app is up.
    #[serde(rename = "hello")]
    Hello,
    #[serde(rename = "status")]
    Status,
    #[serde(rename = "sync")]
    Sync { history: bool },
    #[serde(rename = "items.list")]
    ItemsList {
        scope: Scope,
        unread: bool,
        limit: u32,
        offset: u32,
    },
    #[serde(rename = "items.get")]
    ItemsGet { ids: Vec<String> },
    #[serde(rename = "items.mark")]
    ItemsMark { ids: Vec<String>, mark: Mark },
    #[serde(rename = "items.read_all")]
    ItemsReadAll {
        scope: Scope,
        /// Only articles published up to this moment (milliseconds).
        before: Option<i64>,
        dry_run: bool,
    },
    #[serde(rename = "items.delete")]
    ItemsDelete { ids: Vec<String> },
    #[serde(rename = "items.fetch")]
    ItemsFetch { id: String },
    #[serde(rename = "items.tag")]
    ItemsTag {
        id: String,
        names: Vec<String>,
        on: bool,
    },
    #[serde(rename = "doc")]
    Doc {
        id: String,
        format: DocFormat,
        /// First and last page of a PDF, counted from 1.
        pages: Option<(u32, u32)>,
    },
    #[serde(rename = "add.url")]
    AddUrl { url: String },
    #[serde(rename = "add.file")]
    AddFile { path: String },
    #[serde(rename = "add.text")]
    AddText {
        title: Option<String>,
        body: String,
        html: bool,
        url: Option<String>,
    },
    #[serde(rename = "feeds.list")]
    FeedsList,
    #[serde(rename = "feeds.add")]
    FeedsAdd { url: String, folder: Option<String> },
    #[serde(rename = "feeds.remove")]
    FeedsRemove { id: String },
    #[serde(rename = "folders")]
    Folders,
    #[serde(rename = "search")]
    Search {
        query: String,
        limit: u32,
        /// Only this device's index, without asking the server.
        local: bool,
    },
    #[serde(rename = "progress.get")]
    ProgressGet { id: String },
    #[serde(rename = "progress.set")]
    ProgressSet { id: String, to: ProgressTarget },
    #[serde(rename = "tags.list")]
    TagsList,
    #[serde(rename = "tags.add")]
    TagsAdd { name: String },
    #[serde(rename = "tags.rename")]
    TagsRename { name: String, to: String },
    #[serde(rename = "tags.remove")]
    TagsRemove { name: String },
    #[serde(rename = "series.list")]
    SeriesList,
    #[serde(rename = "series.get")]
    SeriesGet { id: String },
    #[serde(rename = "series.detect")]
    SeriesDetect { url: String },
    #[serde(rename = "series.create")]
    SeriesCreate { url: String, title: Option<String> },
    #[serde(rename = "series.add")]
    SeriesAdd {
        id: String,
        item: String,
        /// Position among the chapters, from 0; at the end when left out.
        index: Option<u32>,
    },
    #[serde(rename = "series.remove")]
    SeriesRemove { item: String },
    #[serde(rename = "series.rename")]
    SeriesRename { id: String, title: String },
    #[serde(rename = "series.delete")]
    SeriesDelete { id: String },
    #[serde(rename = "collection")]
    Collection { kind: Option<String> },
    #[serde(rename = "summary")]
    Summary {
        id: String,
        /// Ask the model when there is no summary yet.
        generate: bool,
        /// Ask the model even when there is one.
        force: bool,
    },
    #[serde(rename = "translation")]
    Translation { id: String },
    #[serde(rename = "export.epub")]
    ExportEpub { id: String, path: String },
}

impl Request {
    /// Whether lists on screen may be out of date once this has run.
    pub fn mutates(&self) -> bool {
        use Request::*;
        match self {
            Hello
            | Status
            | ItemsList { .. }
            | ItemsGet { .. }
            | Doc { .. }
            | FeedsList
            | Folders
            | Search { .. }
            | ProgressGet { .. }
            | TagsList
            | SeriesList
            | SeriesGet { .. }
            | SeriesDetect { .. }
            | Collection { .. }
            | Translation { .. }
            | ExportEpub { .. } => false,
            ItemsReadAll { dry_run, .. } => !dry_run,
            Summary { generate, .. } => *generate,
            Sync { .. }
            | ItemsMark { .. }
            | ItemsDelete { .. }
            | ItemsFetch { .. }
            | ItemsTag { .. }
            | AddUrl { .. }
            | AddFile { .. }
            | AddText { .. }
            | FeedsAdd { .. }
            | FeedsRemove { .. }
            | ProgressSet { .. }
            | TagsAdd { .. }
            | TagsRename { .. }
            | TagsRemove { .. }
            | SeriesCreate { .. }
            | SeriesAdd { .. }
            | SeriesRemove { .. }
            | SeriesRename { .. }
            | SeriesDelete { .. } => true,
        }
    }
}

/// Which items a list or "mark all read" covers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Subscription articles and saved pages.
    #[default]
    All,
    Starred,
    Saved,
    /// Started but not finished, most recently read first.
    Continue,
    /// A subscription, by ID.
    Feed(String),
    /// A folder of subscriptions, by name.
    Folder(String),
    /// The chapters of a series, by ID.
    Series(String),
    /// A tag, by name.
    Tag(String),
    /// Title contains the text.
    Title(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Read,
    Unread,
    Star,
    Unstar,
    Finish,
    Unfinish,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocFormat {
    #[default]
    Markdown,
    /// Plain text, a blank line between blocks.
    Text,
    /// The document model as it is stored.
    Json,
    /// One line per block: its ID, kind and text.
    Blocks,
    /// Counts of blocks by kind and an excerpt.
    Summary,
}

/// Where `progress.set` puts the reading position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressTarget {
    /// In a block, `offset` UTF-16 code units into its text.
    Block { block: String, offset: u32 },
    /// A page of a PDF, counted from 1.
    Page { page: u32 },
    /// Read to the end.
    End,
}

/// One line of an answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frame {
    Progress(Progress),
    Ok(serde_json::Value),
    Error(ControlError),
}

/// How far a long request has got.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    /// `uploading`, `downloading`, `fetching`, `history`, `export`.
    pub phase: String,
    pub done: u64,
    pub total: Option<u64>,
}

/// An error with a name tools can act on. Engine errors keep their own
/// names (`not_found`, `offline`, `invalid`, …); the ones this layer adds
/// are `bad_request`, `version_mismatch`, `app_unreachable`, `no_content`
/// and `internal`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct ControlError {
    pub code: String,
    pub message: String,
}

impl ControlError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        ControlError {
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn not_found(what: &str, id: &str) -> Self {
        Self::new("not_found", format!("no {what} with ID {id}"))
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }
}

impl From<eddy_core::EngineError> for ControlError {
    fn from(e: eddy_core::EngineError) -> Self {
        ControlError::new(e.code(), e.to_string())
    }
}

impl From<serde_json::Error> for ControlError {
    fn from(e: serde_json::Error) -> Self {
        ControlError::new("internal", e.to_string())
    }
}
