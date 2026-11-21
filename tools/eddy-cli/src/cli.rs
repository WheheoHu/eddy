//! The commands and their arguments.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use eddy_control::{DocFormat, Mark, ProgressTarget, Request, Scope};

#[derive(Debug, Parser)]
#[command(
    name = "eddy",
    // Replaced when the program starts by one that names the commit.
    version = env!("CARGO_PKG_VERSION"),
    about = "Eddy from the terminal: add things to read, manage subscriptions, look up items and their text, read and set progress.",
    long_about = "Eddy from the terminal: add things to read, manage subscriptions, look up items and \
their text, read and set progress.\n\n\
Installed with the app, it works on your library through Eddy, starting it in the background \
when it is not running. With --data DIR it works on a directory of its own instead, in this \
process, without the app.\n\n\
Every command prints text for reading, or with --json one JSON object. `eddy guide` explains \
the whole thing to an agent.",
    after_help = "Exit status: 0 done, 1 the command failed, 2 it was not understood, 3 Eddy could not be reached.",
    propagate_version = true,
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Work on the data directory DIR in this process, without the app
    #[arg(long, global = true, value_name = "DIR", conflicts_with = "app")]
    pub data: Option<PathBuf>,
    /// Work through the running app (the default for the copy inside Eddy.app)
    #[arg(long, global = true)]
    pub app: bool,
    /// Print the result as one JSON object
    #[arg(long, global = true)]
    pub json: bool,
    /// How long to wait for Eddy to start
    #[arg(long, global = true, value_name = "SECONDS", default_value_t = 30)]
    pub timeout: u64,
    /// Say nothing about progress
    #[arg(long, short, global = true)]
    pub quiet: bool,
    #[command(subcommand)]
    pub command: Command,
}

// Parsed once per run: the size of the largest command does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
pub enum Command {
    /// How Eddy is set up and how sync stands
    Status,
    /// Fetch new articles and send local changes
    Sync {
        /// Do not go on to fetch older articles
        #[arg(long)]
        no_history: bool,
    },
    /// List items, look at them, mark and delete them
    #[command(args_conflicts_with_subcommands = true)]
    Items {
        #[command(subcommand)]
        command: Option<ItemsCommand>,
        #[command(flatten)]
        list: ListArgs,
    },
    /// An item's text
    Doc {
        id: String,
        #[arg(long, value_enum, default_value_t = Format::Markdown)]
        format: Format,
        /// Only these pages of a PDF: 3 or 3-7
        #[arg(long, value_name = "A-B", value_parser = parse_pages)]
        pages: Option<(u32, u32)>,
    },
    /// Save a page, a file or text
    #[command(subcommand)]
    Add(AddCommand),
    /// Subscriptions
    Feeds {
        #[command(subcommand)]
        command: Option<FeedsCommand>,
    },
    /// The folders subscriptions are filed in
    Folders,
    /// Full-text search
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
        /// Only what is on this device, without asking the server
        #[arg(long)]
        local: bool,
    },
    /// Where reading stopped
    #[command(subcommand)]
    Progress(ProgressCommand),
    /// Tags
    Tags {
        #[command(subcommand)]
        command: Option<TagsCommand>,
    },
    /// Books and collected web series
    Series {
        #[command(subcommand)]
        command: Option<SeriesCommand>,
    },
    /// Everything saved or imported: books, PDFs, pages, series
    Collection {
        /// book, pdf, web_page, media or web_series
        #[arg(long)]
        kind: Option<String>,
    },
    /// An item's summary
    Summary {
        id: String,
        /// Ask the model when there is none yet (uses your API key)
        #[arg(long)]
        generate: bool,
        /// Ask the model even when there is one
        #[arg(long, requires = "generate")]
        force: bool,
    },
    /// An item's translation, when it has been translated
    Translation { id: String },
    /// Write items out as files
    #[command(subcommand)]
    Export(ExportCommand),
    /// How to use this command, written for an agent
    Guide,
    /// Tell agents such as Claude Code and Codex that this command exists
    #[command(
        subcommand,
        long_about = "Tell agents such as Claude Code and Codex that this command exists.\n\n\
A skill is installed where agents look for them: the shared ~/.agents/skills, which Codex, \
Gemini CLI, Cursor, GitHub Copilot, opencode and most others read, and Claude Code's own \
~/.claude/skills. It is a few lines that send the agent to `eddy guide`, so what agents are \
told always matches the installed Eddy. Agents pick it up in their next session.\n\n\
The same is done by Set Up Agents in Eddy's Settings."
    )]
    Skill(SkillCommand),

    /// Pair this data directory with a server (only with --data)
    Connect { url: String, code: String },
    /// Leave the server, keeping local data (only with --data)
    Disconnect,
    /// Leave the server and erase this data directory (only with --data)
    SignOut {
        /// Also give up changes that could not be sent
        #[arg(long)]
        discard_unsent: bool,
    },
    /// Set this data directory's FreshRSS account (only with --data)
    Freshrss {
        url: String,
        user: String,
        password: String,
    },
    /// Set the FreshRSS account the server manages (only with --data)
    ServerAccount {
        url: String,
        user: String,
        password: String,
    },
    /// Checks for developing Eddy (only with --data)
    #[command(subcommand, hide = true)]
    Dev(DevCommand),

    // The spellings from when this was a development tool.
    #[command(hide = true)]
    Save { url: String },
    #[command(hide = true)]
    Import { path: PathBuf },
    #[command(subcommand, hide = true)]
    Feed(LegacyFeedCommand),
    #[command(hide = true)]
    Star { id: String },
    #[command(hide = true)]
    Tag { name: String },
    #[command(hide = true)]
    Text { id: String },
    #[command(hide = true)]
    Fulltext { id: String },
}

/// Which items; one of these at most.
#[derive(Debug, Default, Args)]
#[group(multiple = false)]
pub struct ScopeArgs {
    /// Subscription articles and saved pages (the default for a list)
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub starred: bool,
    /// Pages, files and text saved by hand
    #[arg(long)]
    pub saved: bool,
    /// Started and not finished, most recently read first
    #[arg(long = "continue")]
    pub continuing: bool,
    /// A subscription (`eddy feeds` lists the IDs)
    #[arg(long, value_name = "ID")]
    pub feed: Option<String>,
    /// A folder of subscriptions
    #[arg(long, value_name = "NAME")]
    pub folder: Option<String>,
    /// The chapters of a book or series
    #[arg(long, value_name = "ID")]
    pub series: Option<String>,
    #[arg(long, value_name = "NAME")]
    pub tag: Option<String>,
    /// Title contains TEXT
    #[arg(long, value_name = "TEXT")]
    pub title: Option<String>,
}

impl ScopeArgs {
    /// The scope named, if one was.
    pub fn named(&self) -> Option<Scope> {
        if self.all {
            Some(Scope::All)
        } else if self.starred {
            Some(Scope::Starred)
        } else if self.saved {
            Some(Scope::Saved)
        } else if self.continuing {
            Some(Scope::Continue)
        } else if let Some(id) = &self.feed {
            Some(Scope::Feed(id.clone()))
        } else if let Some(name) = &self.folder {
            Some(Scope::Folder(name.clone()))
        } else if let Some(id) = &self.series {
            Some(Scope::Series(id.clone()))
        } else if let Some(name) = &self.tag {
            Some(Scope::Tag(name.clone()))
        } else {
            self.title.clone().map(Scope::Title)
        }
    }
}

#[derive(Debug, Default, Args)]
pub struct ListArgs {
    #[command(flatten)]
    pub scope: ScopeArgs,
    /// Only unread items
    #[arg(long)]
    pub unread: bool,
    #[arg(long, default_value_t = 50)]
    pub limit: u32,
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
}

impl ListArgs {
    fn request(&self) -> Request {
        Request::ItemsList {
            scope: self.scope.named().unwrap_or_default(),
            unread: self.unread,
            limit: self.limit,
            offset: self.offset,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum ItemsCommand {
    /// List items, newest first: ID, date, state (u unread, * starred, f finished), title, feed
    List(ListArgs),
    /// Everything about items, with their tags
    Get {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Mark as read
    Read {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Mark as unread, which also resets the reading position
    Unread {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    Star {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    Unstar {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Mark as read to the end
    Finish {
        #[arg(required = true)]
        ids: Vec<String>,
        /// Mark as not finished instead
        #[arg(long)]
        undo: bool,
    },
    /// Mark every unread subscription article in a scope as read
    ReadAll {
        #[command(flatten)]
        scope: ScopeArgs,
        /// Only articles published up to this moment (milliseconds since 1970)
        #[arg(long, value_name = "MS")]
        before: Option<i64>,
        /// Only count what would be marked
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete saved pages, PDFs, or whole series, on every device. Not undoable
    Delete {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Fetch the full text of an item from its page
    Fetch { id: String },
    /// Put tags on an item, making the ones that do not exist
    Tag {
        id: String,
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Take tags off an item
    Untag {
        id: String,
        #[arg(required = true)]
        names: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Markdown,
    /// Plain text
    Text,
    /// The document model: blocks with IDs, runs, styles
    Json,
    /// One line per block: its ID, kind and text
    Blocks,
    /// What the document is made of
    Summary,
}

impl From<Format> for DocFormat {
    fn from(format: Format) -> Self {
        match format {
            Format::Markdown => DocFormat::Markdown,
            Format::Text => DocFormat::Text,
            Format::Json => DocFormat::Json,
            Format::Blocks => DocFormat::Blocks,
            Format::Summary => DocFormat::Summary,
        }
    }
}

fn parse_pages(text: &str) -> Result<(u32, u32), String> {
    let (first, last) = text.split_once('-').unwrap_or((text, text));
    match (first.trim().parse::<u32>(), last.trim().parse::<u32>()) {
        (Ok(first), Ok(last)) if first >= 1 && first <= last => Ok((first, last)),
        _ => Err("pages are written 3 or 3-7, counted from 1".into()),
    }
}

#[derive(Debug, Subcommand)]
pub enum AddCommand {
    /// Save a web page (or a link to an image or video)
    Url { url: String },
    /// Import an EPUB or a PDF
    File { path: PathBuf },
    /// Save Markdown or HTML as a document, from FILE or standard input
    Text {
        /// Its title; otherwise the first heading, or the opening words
        #[arg(long)]
        title: Option<String>,
        #[arg(long, value_enum, default_value_t = TextKind::Markdown)]
        format: TextKind,
        /// Where the text came from: relative links resolve against it
        #[arg(long)]
        url: Option<String>,
        /// The file to read; standard input when left out or `-`
        file: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum TextKind {
    Markdown,
    Html,
}

#[derive(Debug, Subcommand)]
pub enum FeedsCommand {
    /// Subscriptions with their folder and unread count
    List,
    /// Subscribe to a feed, or to the feed a page names
    Add {
        url: String,
        /// File it in this folder, made if it does not exist
        #[arg(long)]
        folder: Option<String>,
    },
    /// Unsubscribe; the feed leaves with its articles
    Remove { id: String },
}

#[derive(Debug, Subcommand)]
pub enum LegacyFeedCommand {
    Add { url: String, folder: Option<String> },
    Remove { id: String },
    List,
}

#[derive(Debug, Subcommand)]
pub enum ProgressCommand {
    /// How far an item has been read and where reading stopped
    Get { id: String },
    /// Move the reading position
    Set {
        id: String,
        #[command(flatten)]
        to: PositionArgs,
        /// With --block: UTF-16 code units into the block's text
        #[arg(long, value_name = "N")]
        offset: Option<u32>,
    },
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub struct PositionArgs {
    /// A block (`eddy doc ID --format blocks` lists them)
    #[arg(long, value_name = "ID")]
    pub block: Option<String>,
    /// A page of a PDF, counted from 1
    #[arg(long, value_name = "N")]
    pub page: Option<u32>,
    /// Read to the end
    #[arg(long)]
    pub end: bool,
}

#[derive(Debug, Subcommand)]
pub enum TagsCommand {
    /// Every tag's name
    List,
    Add {
        name: String,
    },
    Rename {
        name: String,
        to: String,
    },
    /// Delete a tag, taking it off every item
    Remove {
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum SeriesCommand {
    List,
    /// A series and its chapters
    Get {
        id: String,
    },
    /// Look for a table of contents on a page
    Detect {
        url: String,
    },
    /// Collect the chapters a page's table of contents lists
    Create {
        url: String,
        #[arg(long)]
        title: Option<String>,
    },
    /// Make an item a chapter of a series
    Add {
        id: String,
        item: String,
        /// Its place among the chapters, from 0; the end when left out
        #[arg(long)]
        index: Option<u32>,
    },
    /// Take an item out of its series
    Remove {
        item: String,
    },
    Rename {
        id: String,
        title: String,
    },
    /// Delete a series and its chapters, on every device. Not undoable
    Delete {
        id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ExportCommand {
    /// Write an item, a book or a series as an EPUB
    Epub {
        id: String,
        #[arg(long, short, value_name = "PATH")]
        output: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// Which agents are on this Mac and whether the skill is installed for them
    Status,
    /// Install the skill for the agents on this Mac, or bring it up to date
    Setup {
        /// Say what would change without changing anything
        #[arg(long)]
        dry_run: bool,
        /// Replace a skill named eddy that Eddy did not install
        #[arg(long)]
        force: bool,
    },
    /// Remove the skill wherever Eddy installed it
    Remove,
    /// Print the skill that is installed
    Show,
}

#[derive(Debug, Subcommand)]
pub enum DevCommand {
    /// Fetch one image the way the reader does
    Image { url: String, page: Option<String> },
    /// Resolve an item's media
    Media { id: String },
    /// Turn a folder's Gallery on and tally what its items resolve to
    GalleryProbe {
        folder: String,
        #[arg(default_value_t = 20)]
        count: u32,
    },
}

/// What a command comes down to.
#[derive(Debug)]
pub enum Action {
    /// A request, carried out here or by the app.
    Request(Request),
    /// Text to save: read where the command runs, then sent as a request.
    AddText {
        title: Option<String>,
        html: bool,
        url: Option<String>,
        file: Option<PathBuf>,
    },
    /// A file to import: staged where the app can read it.
    AddFile(PathBuf),
    /// A file to write: collected from where the app can write it.
    ExportEpub {
        id: String,
        output: PathBuf,
    },
    /// Work on an engine in this process only.
    Local(Local),
    Guide,
    /// The skill for agents: no engine and no app involved.
    Skill(SkillCommand),
}

#[derive(Debug)]
pub enum Local {
    Connect {
        url: String,
        code: String,
    },
    Disconnect,
    SignOut {
        discard_unsent: bool,
    },
    Freshrss {
        url: String,
        user: String,
        password: String,
    },
    ServerAccount {
        url: String,
        user: String,
        password: String,
    },
    Dev(DevCommand),
}

impl Command {
    pub fn action(self) -> Result<Action, String> {
        let request = |request| Ok(Action::Request(request));
        let mark = |ids, mark| Ok(Action::Request(Request::ItemsMark { ids, mark }));
        match self {
            Command::Status => request(Request::Status),
            Command::Sync { no_history } => request(Request::Sync {
                history: !no_history,
            }),
            Command::Items { command, list } => match command {
                None => request(list.request()),
                Some(ItemsCommand::List(list)) => request(list.request()),
                Some(ItemsCommand::Get { ids }) => request(Request::ItemsGet { ids }),
                Some(ItemsCommand::Read { ids }) => mark(ids, Mark::Read),
                Some(ItemsCommand::Unread { ids }) => mark(ids, Mark::Unread),
                Some(ItemsCommand::Star { ids }) => mark(ids, Mark::Star),
                Some(ItemsCommand::Unstar { ids }) => mark(ids, Mark::Unstar),
                Some(ItemsCommand::Finish { ids, undo }) => {
                    mark(ids, if undo { Mark::Unfinish } else { Mark::Finish })
                }
                Some(ItemsCommand::ReadAll {
                    scope,
                    before,
                    dry_run,
                }) => match scope.named() {
                    Some(scope) => request(Request::ItemsReadAll {
                        scope,
                        before,
                        dry_run,
                    }),
                    None => Err(
                        "say which articles: --all, --feed ID, --folder NAME, --tag NAME, …".into(),
                    ),
                },
                Some(ItemsCommand::Delete { ids }) => request(Request::ItemsDelete { ids }),
                Some(ItemsCommand::Fetch { id }) => request(Request::ItemsFetch { id }),
                Some(ItemsCommand::Tag { id, names }) => request(Request::ItemsTag {
                    id,
                    names,
                    on: true,
                }),
                Some(ItemsCommand::Untag { id, names }) => request(Request::ItemsTag {
                    id,
                    names,
                    on: false,
                }),
            },
            Command::Doc { id, format, pages } => request(Request::Doc {
                id,
                format: format.into(),
                pages,
            }),
            Command::Add(AddCommand::Url { url }) | Command::Save { url } => {
                request(Request::AddUrl { url })
            }
            Command::Add(AddCommand::File { path }) | Command::Import { path } => {
                Ok(Action::AddFile(path))
            }
            Command::Add(AddCommand::Text {
                title,
                format,
                url,
                file,
            }) => Ok(Action::AddText {
                title,
                html: format == TextKind::Html,
                url,
                file: file.filter(|f| f.as_os_str() != "-"),
            }),
            Command::Feeds { command } => match command {
                None | Some(FeedsCommand::List) => request(Request::FeedsList),
                Some(FeedsCommand::Add { url, folder }) => {
                    request(Request::FeedsAdd { url, folder })
                }
                Some(FeedsCommand::Remove { id }) => request(Request::FeedsRemove { id }),
            },
            Command::Feed(LegacyFeedCommand::List) => request(Request::FeedsList),
            Command::Feed(LegacyFeedCommand::Add { url, folder }) => {
                request(Request::FeedsAdd { url, folder })
            }
            Command::Feed(LegacyFeedCommand::Remove { id }) => request(Request::FeedsRemove { id }),
            Command::Folders => request(Request::Folders),
            Command::Search {
                query,
                limit,
                local,
            } => request(Request::Search {
                query,
                limit,
                local,
            }),
            Command::Progress(ProgressCommand::Get { id }) => request(Request::ProgressGet { id }),
            Command::Progress(ProgressCommand::Set { id, to, offset }) => {
                // Not something the argument parser can be told: it waives
                // a requirement on --block once --page or --end is given.
                if offset.is_some() && to.block.is_none() {
                    return Err("--offset goes with --block".into());
                }
                let to = match (to.block, to.page) {
                    (Some(block), _) => ProgressTarget::Block {
                        block,
                        offset: offset.unwrap_or(0),
                    },
                    (None, Some(page)) => ProgressTarget::Page { page },
                    (None, None) => ProgressTarget::End,
                };
                request(Request::ProgressSet { id, to })
            }
            Command::Tags { command } => match command {
                None | Some(TagsCommand::List) => request(Request::TagsList),
                Some(TagsCommand::Add { name }) => request(Request::TagsAdd { name }),
                Some(TagsCommand::Rename { name, to }) => request(Request::TagsRename { name, to }),
                Some(TagsCommand::Remove { name }) => request(Request::TagsRemove { name }),
            },
            Command::Tag { name } => request(Request::TagsAdd { name }),
            Command::Series { command } => match command {
                None | Some(SeriesCommand::List) => request(Request::SeriesList),
                Some(SeriesCommand::Get { id }) => request(Request::SeriesGet { id }),
                Some(SeriesCommand::Detect { url }) => request(Request::SeriesDetect { url }),
                Some(SeriesCommand::Create { url, title }) => {
                    request(Request::SeriesCreate { url, title })
                }
                Some(SeriesCommand::Add { id, item, index }) => {
                    request(Request::SeriesAdd { id, item, index })
                }
                Some(SeriesCommand::Remove { item }) => request(Request::SeriesRemove { item }),
                Some(SeriesCommand::Rename { id, title }) => {
                    request(Request::SeriesRename { id, title })
                }
                Some(SeriesCommand::Delete { id }) => request(Request::SeriesDelete { id }),
            },
            Command::Collection { kind } => request(Request::Collection { kind }),
            Command::Summary {
                id,
                generate,
                force,
            } => request(Request::Summary {
                id,
                generate,
                force,
            }),
            Command::Translation { id } => request(Request::Translation { id }),
            Command::Export(ExportCommand::Epub { id, output }) => {
                Ok(Action::ExportEpub { id, output })
            }
            Command::Guide => Ok(Action::Guide),
            Command::Skill(command) => Ok(Action::Skill(command)),
            Command::Star { id } => mark(vec![id], Mark::Star),
            Command::Text { id } => request(Request::Doc {
                id,
                format: DocFormat::Blocks,
                pages: None,
            }),
            Command::Fulltext { id } => request(Request::ItemsFetch { id }),
            Command::Connect { url, code } => Ok(Action::Local(Local::Connect { url, code })),
            Command::Disconnect => Ok(Action::Local(Local::Disconnect)),
            Command::SignOut { discard_unsent } => {
                Ok(Action::Local(Local::SignOut { discard_unsent }))
            }
            Command::Freshrss {
                url,
                user,
                password,
            } => Ok(Action::Local(Local::Freshrss {
                url,
                user,
                password,
            })),
            Command::ServerAccount {
                url,
                user,
                password,
            } => Ok(Action::Local(Local::ServerAccount {
                url,
                user,
                password,
            })),
            Command::Dev(dev) => Ok(Action::Local(Local::Dev(dev))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(line: &str) -> Cli {
        Cli::try_parse_from(line.split_whitespace()).unwrap_or_else(|e| panic!("{line}: {e}"))
    }

    fn request(line: &str) -> Request {
        match parse(line).command.action().unwrap() {
            Action::Request(request) => request,
            other => panic!("{line}: {other:?}"),
        }
    }

    #[test]
    fn the_commands_are_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn data_comes_first_or_anywhere() {
        // The server smoke test puts it first.
        let first = parse("eddy-cli --data build/smoke/a tags");
        assert_eq!(
            first.data.as_deref(),
            Some(std::path::Path::new("build/smoke/a"))
        );
        let last = parse("eddy tags --data x --json");
        assert!(last.json && last.data.is_some());
        assert!(Cli::try_parse_from(["eddy", "--data", "x", "--app", "tags"]).is_err());
        assert!(Cli::try_parse_from(["eddy"]).is_err());
    }

    #[test]
    fn items_list_by_default_and_take_one_scope() {
        assert_eq!(
            request("eddy items"),
            Request::ItemsList {
                scope: Scope::All,
                unread: false,
                limit: 50,
                offset: 0
            }
        );
        assert_eq!(
            request("eddy items --unread --folder Tech --limit 5 --offset 10"),
            Request::ItemsList {
                scope: Scope::Folder("Tech".into()),
                unread: true,
                limit: 5,
                offset: 10
            }
        );
        assert_eq!(
            request("eddy items list --continue"),
            Request::ItemsList {
                scope: Scope::Continue,
                unread: false,
                limit: 50,
                offset: 0
            }
        );
        assert!(Cli::try_parse_from(["eddy", "items", "--starred", "--saved"]).is_err());
        assert_eq!(
            request("eddy items finish a b --undo"),
            Request::ItemsMark {
                ids: vec!["a".into(), "b".into()],
                mark: Mark::Unfinish
            }
        );
        assert!(Cli::try_parse_from(["eddy", "items", "read"]).is_err());
    }

    #[test]
    fn marking_everything_read_needs_to_be_told_what() {
        assert!(parse("eddy items read-all").command.action().is_err());
        assert_eq!(
            request("eddy items read-all --all --dry-run"),
            Request::ItemsReadAll {
                scope: Scope::All,
                before: None,
                dry_run: true
            }
        );
    }

    #[test]
    fn documents_take_a_format_and_pages() {
        assert_eq!(
            request("eddy doc x"),
            Request::Doc {
                id: "x".into(),
                format: DocFormat::Markdown,
                pages: None
            }
        );
        assert_eq!(
            request("eddy doc x --format json --pages 3-7"),
            Request::Doc {
                id: "x".into(),
                format: DocFormat::Json,
                pages: Some((3, 7))
            }
        );
        assert_eq!(parse_pages("4"), Ok((4, 4)));
        assert!(parse_pages("0-2").is_err());
        assert!(parse_pages("7-3").is_err());
    }

    #[test]
    fn progress_is_set_to_exactly_one_place() {
        assert_eq!(
            request("eddy progress set x --block b3 --offset 12"),
            Request::ProgressSet {
                id: "x".into(),
                to: ProgressTarget::Block {
                    block: "b3".into(),
                    offset: 12
                }
            }
        );
        assert_eq!(
            request("eddy progress set x --page 4"),
            Request::ProgressSet {
                id: "x".into(),
                to: ProgressTarget::Page { page: 4 }
            }
        );
        assert_eq!(
            request("eddy progress set x --end"),
            Request::ProgressSet {
                id: "x".into(),
                to: ProgressTarget::End
            }
        );
        for wrong in ["eddy progress set x", "eddy progress set x --page 1 --end"] {
            assert!(
                Cli::try_parse_from(wrong.split_whitespace()).is_err(),
                "{wrong}"
            );
        }
        assert!(
            parse("eddy progress set x --page 1 --offset 3")
                .command
                .action()
                .is_err()
        );
    }

    #[test]
    fn the_old_spellings_still_work() {
        assert_eq!(
            request("eddy-cli save https://example.com/a"),
            request("eddy add url https://example.com/a")
        );
        assert_eq!(
            request("eddy-cli feed add https://example.com/feed Tech"),
            request("eddy feeds add https://example.com/feed --folder Tech")
        );
        assert_eq!(request("eddy-cli feed list"), request("eddy feeds"));
        assert_eq!(
            request("eddy-cli feed remove f1"),
            request("eddy feeds remove f1")
        );
        assert_eq!(
            request("eddy-cli tag Later"),
            request("eddy tags add Later")
        );
        assert_eq!(request("eddy-cli tags"), Request::TagsList);
        assert_eq!(request("eddy-cli star x"), request("eddy items star x"));
        assert_eq!(
            request("eddy-cli fulltext x"),
            request("eddy items fetch x")
        );
        assert_eq!(
            request("eddy-cli text x"),
            request("eddy doc x --format blocks")
        );
        assert!(matches!(
            parse("eddy-cli import book.epub").command.action().unwrap(),
            Action::AddFile(_)
        ));
        assert!(matches!(
            parse("eddy-cli --data d sign-out --discard-unsent")
                .command
                .action()
                .unwrap(),
            Action::Local(Local::SignOut {
                discard_unsent: true
            })
        ));
        assert!(matches!(
            parse("eddy-cli --data d connect http://127.0.0.1:1 CODE")
                .command
                .action()
                .unwrap(),
            Action::Local(Local::Connect { .. })
        ));
    }

    #[test]
    fn text_comes_from_a_file_or_standard_input() {
        let Action::AddText {
            file, html, title, ..
        } = parse("eddy add text --title Notes --format html -")
            .command
            .action()
            .unwrap()
        else {
            panic!()
        };
        assert_eq!((file, html, title.as_deref()), (None, true, Some("Notes")));
        let Action::AddText { file, html, .. } =
            parse("eddy add text notes.md").command.action().unwrap()
        else {
            panic!()
        };
        assert_eq!((file, html), (Some(PathBuf::from("notes.md")), false));
    }
}
