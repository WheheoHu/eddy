# Eddy

Eddy is a local-first personal reader for news feeds, saved web pages, books and PDFs. Subscriptions come from FreshRSS (or any Google Reader-compatible service), anything on the web can be saved for later, and reading progress is remembered the way a book app remembers it.

- **One Rust core, native interfaces.** All business logic lives in a Rust workspace and reaches the UI through UniFFI. macOS (SwiftUI + TextKit 2) comes first; iOS and Android follow.
- **Works without a server.** Every client is complete on its own. The optional `eddy-server` adds cross-device sync and search over the full history.
- **Processing stays on your device.** Full-text extraction, truncation detection, fetch strategies and AI translation, summaries and tagging all run on the client. API keys stay in the system keychain.

Eddy is in early development (0.1.0). The macOS app is the only interface so far, and its UI is English only.

## Features

- **Feeds** — a three-pane reader for FreshRSS subscriptions, with read state, stars and reading position kept per article. Feeds are added from Eddy, into a folder or none; FreshRSS does the subscribing.
- **Collection** — saved web pages, web series built from a table of contents, imported EPUB books and PDFs, side by side with your feeds.
- **Reflowed reading** — articles, EPUB chapters and PDF text are converted into one block-based document model and laid out with TextKit 2: formulas, code, tables, images, bilingual text, and Chinese typography.
- **Gallery** — a separate view for image- and video-heavy feeds and saved media links.
- **AI on the client** — paragraph translation, summaries and tags from a controlled vocabulary, against any OpenAI-compatible or Gemini-native endpoint you configure.
- **Video to book** — turns a talk or presentation (an online link or a local file) into a multi-chapter illustrated EPUB in your Collection.
- **Save from Chrome** — a browser extension hands the page you are looking at, including pages you are signed in to, over to Eddy, even while Eddy is closed. It also lists the feeds a page has, and the ones [RSSHub](https://docs.rsshub.app) can make for it, and opens Eddy's Add Feed sheet on the one you pick.
- **Search** — full-text search with Chinese word segmentation; local data on the client, the whole history on the server.

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/` | The Rust workspace (see below) |
| `apple/` | The macOS app (an XcodeGen project) and the `EddyKit` Swift package |
| `extensions/chrome/` | The "Save to Eddy" Chrome extension |
| `deploy/` | Dockerfile and compose file for `eddy-server` |
| `scripts/` | Build and install scripts |
| `tools/` | `eddy-cli`, a headless developer tool, and the UniFFI binding generator |
| `docs/adr/` | Architecture decision records (in Chinese) |

### Crates

| Crate | Role |
| --- | --- |
| `eddy-model` | Domain types shared by every crate: identifiers, the document model, locators, items, the operation log |
| `eddy-doc` | Converts HTML into the reflowable document model; translation segments, locators and re-anchoring |
| `eddy-store` | SQLite store: items, content versions, the operation log with last-writer-wins merging, content-addressed blobs |
| `eddy-source-greader` | Google Reader API client (FreshRSS, Miniflux and compatible services) |
| `eddy-extract` | Full-text extraction (site rules, then Readability), truncation and series detection |
| `eddy-fetch` | Generic page-fetching pipeline and the rule pack format; contains no site rules |
| `eddy-media` | Resolves an item's link into images and videos that can be shown directly |
| `eddy-epub` | EPUB container parsing for imports |
| `eddy-pdf` | Reads PDF text and images with a statically linked pdfium |
| `eddy-ai` | Translation, summaries and tagging over OpenAI-compatible and Gemini-native endpoints |
| `eddy-video` | The video-to-book pipeline, shared by the client and the server |
| `eddy-search` | Full-text search with jieba segmentation |
| `eddy-sync` | The sync protocol between clients and `eddy-server` |
| `eddy-core` | The engine: owns the store, talks to the upstream, exposes the operations the apps call |
| `eddy-ffi` | The UniFFI surface for Swift and Kotlin |
| `eddy-server` | The optional sync hub |
| `eddy-capture-host` | Chrome native messaging host behind the browser extension |

## Building

Requirements:

- Rust 1.85 or newer (via rustup)
- Xcode 26.3 or newer, on macOS 15 or newer
- XcodeGen: `brew install xcodegen`
- Optional, for video to book: `brew install ffmpeg yt-dlp`

```sh
scripts/fetch-pdfium.sh         # once: downloads the static pdfium library into build/
cargo test --workspace          # Rust tests
scripts/build-mac-app.sh        # builds the Rust core, generates bindings, builds Eddy.app
open apple/DerivedData/Build/Products/Debug/Eddy.app
```

`scripts/build-mac-app.sh` runs `scripts/build-apple.sh` first, which produces `EddyFFI.xcframework` and the Swift bindings. The Xcode project, the XCFramework and the generated bindings are build outputs and are not checked in; rerun the script after changing the exported interface of `eddy-ffi`.

To try the app without touching your real library, point it at a scratch data directory:

```sh
open --env EDDY_DATA_DIR=$PWD/build/app-test apple/DerivedData/Build/Products/Debug/Eddy.app
```

### Windows (Rust core and CLI)

The macOS app is Apple-only, but the Rust core and the headless CLI build on Windows (MSVC toolchain, from a Visual Studio developer shell). `fetch-pdfium.sh` is a bash script, so run it from Git Bash or another shell with `curl`, `unzip` and `sha256sum` (or `shasum`); the cargo commands work from either shell:

```sh
scripts/fetch-pdfium.sh windows   # downloads the Windows pdfium build into build/
cargo build -p eddy-cli
cargo run -p eddy-cli -- save https://example.com/article
```

On Windows pdfium is a shared library: copy `build/pdfium/windows/release/lib/x64/pdfium.dll` next to the built exe (`target/debug/`). `eddy-pdf` loads it from there by full path; the import library in the archive is not needed. Debug builds and tests also fall back to the unpacked copy under `build/`, so `cargo test -p eddy-pdf` works without the copy; release binaries only ever load the DLL next to the exe.

### Chrome extension

Open `chrome://extensions`, turn on Developer mode and load the `extensions/chrome/` folder unpacked. Then click **Connect Chrome** in Eddy's settings.

The toolbar button (⌘⇧E) opens a popup that shows the page and waits: **保存页面到 Eddy** saves it (Return does too), and each feed the page names has a button that opens Eddy's Add Feed sheet on it, where the folder is chosen and the subscription confirmed. The two items in the page's right-click menu save at once. After updating Eddy, press the reload button on the extension's card in `chrome://extensions` so Chrome picks up the new version.

Feeds come from three places: the links in a page's head that name a feed, links in its text that look like one, and RSSHub's rules for the site (the same rules RSSHub Radar uses), which the extension fetches from an RSSHub instance once a day. The instance is `https://rsshub.app` until you set your own in the extension's options (the 设置 link in the popup); the public one is meant for trying things out. The rule matching and the popup have tests that run with Node: `node extensions/tests/radar.test.js && node extensions/tests/popup.test.js`.

### Headless CLI

`tools/eddy-cli` drives the engine without the UI, for checking real network behaviour. It keeps its data and secrets in its own directory under `build/`, separate from the app.

```sh
cargo run -p eddy-cli -- save https://example.com/article
cargo run -p eddy-cli -- items
cargo run -p eddy-cli -- search "query"
```

### Server (optional)

```sh
cd deploy
cp .env.example .env            # set EDDY_MASTER_KEY
docker compose up -d --build    # listens on 127.0.0.1:8787 only
docker compose exec eddy-server eddy-server pairing-code
```

See [`deploy/README.md`](deploy/README.md) for pairing, backups and the video job settings.

## Design notes

The decisions behind the architecture are recorded in [`docs/adr/`](docs/adr/). The short version:

- Local changes are written to an operation log with a hybrid logical clock first, then applied to state. Standalone and server modes share that path, so connecting a server later uploads and merges the log; a server the device has not synced with before gets all of it.
- The server is optional at every moment, not just at setup: while it is unreachable a device keeps working, queues its changes, and fetches new articles from FreshRSS itself.
- The core (the Rust crates, `eddy-server` included) and each app have their own version numbers. What decides whether two sides can sync is the sync protocol version, not either of those.
- Item IDs are deterministic: feed articles derive a UUIDv5 from the source account and the upstream ID; saved pages and series use UUIDv7.
- Text offsets are UTF-16 code units everywhere, matching Swift and Kotlin strings.
- Site-specific fetch rules are never part of this repository. It ships only the generic strategy engine and the rule pack format; rule packs live on your own machine or your own server. See [`crates/eddy-fetch/README.md`](crates/eddy-fetch/README.md).

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
