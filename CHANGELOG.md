# Changelog: core

All notable changes to the Eddy core are recorded here: the engine the apps link, `eddy-server` and `eddy-cli`. The apps have their own changelogs ([Mac](apple/CHANGELOG.md)).

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the core follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Its version is `[workspace.package] version` in `Cargo.toml`; releases are tagged `core-vX.Y.Z`. A change to the sync protocol version is called out, because that, not the version number, decides which servers and devices can sync ([ADR 0009](docs/adr/0009-versions-and-compatibility.md)).

## [Unreleased]

Sync protocol 3. Servers and devices on protocol 1 must be updated together; one on protocol 2 still syncs with one on 3 (see Changed).

### Added

- Windows: the Rust core and `eddy-cli` build and run on Windows (MSVC). `scripts/fetch-pdfium.sh windows` fetches the shared pdfium build, which `eddy-pdf` loads from next to the executable; the macOS app keeps its static link.
- Server: admin page at `/admin`, on when `EDDY_ADMIN_PASSWORD` is set: pairing codes, devices, FreshRSS status and account, video jobs, service status.
- Server: `eddy-server --version`; `/v1/info` reports the version; `/v1/health` checks the database.
- Engine: works with the server offline. Changes queue and upload on their own, search and video job lists answer locally at once, and refresh fetches new articles from FreshRSS directly when the device has the account.
- Engine: status reports whether the server is reachable and how many changes are waiting or were refused.
- Engine: pairing with a server the device has not synced with before uploads its whole history.
- Server: `EDDY_PUBLISH_ADDRESS` in `deploy/.env` chooses the host address the port is published on, for devices on the local network. The default stays `127.0.0.1`.
- A long sync (the first one after pairing, or after a long time offline) reports how far it is: uploading, downloading, fetching articles. `eddy-cli` prints it.
- Server: the log says when a device starts catching up, how far it is, and when it is up to date, and counts each upload. The admin page shows for each device whether it is up to date or how many changes behind.
- Signing a device out and erasing it (`sign_out_and_erase`, `eddy-cli sign-out`): changes not sent yet are sent first, and when some cannot be nothing happens unless losing them is accepted; the server forgets the device; library, downloaded content, search index, rule packs, accounts and every stored secret are erased the next time the data directory is opened.
- Server: `DELETE /v1/device` lets a device remove itself. This needs no protocol change; a device leaving an older server is simply not removed from its list.
- `eddy-cli`: `status`, `disconnect`, `freshrss`, `server-account`, `tag`, `tags`, `star`, `--version`.
- Engine: `overview` reads what the sidebars and shelves are drawn from (status, sources and their counts, series, collection, tags, Gallery folders) in one call, on a read-only connection of its own, so counting a large library in the background no longer keeps other calls waiting.
- `eddy-cli image URL [PAGE]` fetches one image the way the reader does.
- Subscribing to a feed (`add_feed`, `eddy-cli feed add URL [FOLDER]`): FreshRSS subscribes to the address, or to the feed the page there names, and files it in the folder given, making the folder if there is none by that name. A device paired with a server asks the server, which is the one that writes to FreshRSS (`POST /v1/subscriptions`); with the server out of reach the answer is that it is offline, and nothing is queued. The feed reaches every device as a source like any other. This needs no protocol change: a device asking an older server is told the server must be updated.
- Engine: `mark_read` marks items read and finished by hand, leaving the reading position; it is what the apps' Mark as Read does. Opening an item still only marks it read. A Continue Reading query lists the items named in `keep` even once they are finished.
- Engine: `feed_icon` gives the icon of a subscription's site. It is looked for on the device, from the site's address the feed carries: the icons the page names (or the site's front page, when the feed gives its own address as its site's), then `/favicon.ico`; what comes back is only taken if it is an image. The icon is kept in the cache and found again if evicted; a site without one is asked again after a week, and one that can't be reached, the next time. Nothing about icons is synced. The local data format goes to 8.
- Engine: `clear_cache` empties the offline cache now, whatever its limit: articles already read, cached pictures and feed icons go; unread, starred, saved and in-progress content and imported files stay.
- Unsubscribing from a feed (`remove_feed`, `eddy-cli feed remove ID`, with `feed list` for the IDs): FreshRSS unsubscribes, and the feed leaves Eddy with its articles. A device paired with a server asks the server (`DELETE /v1/subscriptions/{id}`); with the server out of reach the answer is that it is offline. A feed FreshRSS no longer has is simply removed here.
- Engine: `save_text` saves Markdown or HTML handed over as it is (notes, a compiled summary) as a document of its own in Saved. Formulas between `$` and `$$` are kept as TeX. It syncs like any saved page and needs no protocol change.
- Engine: a document can be written out as Markdown (`Document::to_markdown`), with tables, nested lists, code and formulas; what is written out saves back as the same document.
- `eddy-cli` is now the command line for people and agents, `eddy` once installed from the app: `items` (list by feed, folder, tag, series, title, unread; `get`, `read`, `unread`, `star`, `finish`, `read-all`, `delete`, `fetch`, `tag`), `doc` (an item's text as Markdown, plain text, blocks or the document model; `--pages` for a PDF), `add url|file|text`, `feeds`, `folders`, `search`, `progress get|set`, `tags`, `series`, `collection`, `summary`, `translation`, `export epub`. Every command takes `--json` and answers with one object; errors carry a code (`not_found`, `offline`, `invalid`, …) and the exit status tells a failed command (1) from one that was not understood (2) and from Eddy not answering (3). `eddy guide` prints instructions for an agent.
- The copy of the command line inside Eddy.app works on the user's library through the app, over a socket in the data directory, and starts the app in the background when it is not running. The app stays the only process that opens the data ([ADR 0011](docs/adr/0011-cli-through-the-app.md)). A copy built from the repository keeps working on `build/cli-data` in its own process and cannot reach the library.
- `eddy skill setup` tells agents that the command exists: it installs a skill in `~/.agents/skills` (read by Codex, Gemini CLI, Cursor, GitHub Copilot, opencode and most other agents) and in Claude Code's `~/.claude/skills`. The skill is a few lines that send the agent to `eddy guide`, so what agents are told always matches the installed Eddy; it names the command's full path for agents that do not have `eddy` on their `PATH`. `eddy skill status` lists the agents found and how the skill stands with each, `remove` takes it out again, `show` prints it. A skill named `eddy` that Eddy did not install is left alone unless `--force` is given.

### Changed

- Video-to-Book: long videos split across chunks now synthesize one concise whole-book overview instead of concatenating per-part summaries and dropping later takeaways. The opening overview chapter now separates the summary, bold-headed key takeaways, and a clickable chapter guide with timestamps.
- `eddy-cli`: `--data DIR` is refused for the app's own data directory, and for a directory an app is running on. `doc ID` prints the text as Markdown (the block counts it used to print are `doc ID --format summary`); `series URL` is `series detect URL`; `gallery FOLDER COUNT` is `dev gallery-probe`; `text ID` lines start with the block's ID; the progress of fetching older articles goes to standard error with the rest. The other commands keep their old spellings and output.
- Sync protocol 2: clients send their protocol version and servers report the oldest they accept; either side tells the other to update when they are too far apart (HTTP 426 from the server).
- A change the server cannot use (unknown kind, wrong value, a clock far ahead) is refused on its own; the rest of the upload goes through.
- Pairing keeps the FreshRSS account on the device and refuses a server that manages a different account.
- The server refreshes FreshRSS right after starting instead of one interval later.
- Data written by a newer version is refused instead of being opened.
- Disconnecting from a server asks it to forget the device, so the device no longer stays listed with a token that still works.
- A first sync is several times faster: pages are larger and compressed, each is stored in one step, and the next downloads while the current one is applied. Pulled pages now say how many changes the server has; this needs no protocol change, and older servers and devices ignore it.
- After a long sync the articles fetched for offline reading are the newest, several at a time.
- Feeds that FreshRSS keeps in its default category ("Uncategorized", or its name in the language FreshRSS is set to) are in no folder, instead of in a folder of that name. The name in another language is learned the first time a feed is added without a folder.
- Refreshing from FreshRSS while the server is offline takes only feeds not listed yet; what the server said of the others stands.
- Trimming the offline cache deletes files several at a time and records them in batches: emptying a cache of two hundred thousand files took a minute and now takes about half that.
- Sync protocol 3: a source that is deleted takes its articles out of every list, starred ones included, as FreshRSS deletes them with the feed; before, they stayed listed as articles of no feed, marked read and unstarred by the next refresh. This holds for a feed unsubscribed from in Eddy and for one removed in FreshRSS itself. Pages saved from such articles stay. A device or server still on protocol 2 syncs as before and goes on listing those articles until it is updated.

### Fixed

- Storing the same content from two threads at once no longer fails one of them with a file error: each write of a blob uses a temporary file of its own.
- A read/star change that loses to a newer one is no longer pushed to FreshRSS, where it used to come back as the newest state.
- Removing a rule pack on one device removes it on the others.
- Revoking a device closes its open live connection.
- The server stops cleanly on SIGTERM.
- Articles stored but not published when the server stopped are ingested again.
- Live sync no longer stacks up connections, times out and pings, and uploads waiting changes when it reconnects.
- Images on hosts that refuse requests without a `Referer` (sspai's among them) load: a refused image is asked for once more the way a browser showing the article would.

[Unreleased]: https://github.com/WheheoHu/eddy/commits/dev
