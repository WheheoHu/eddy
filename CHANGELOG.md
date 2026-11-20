# Changelog: core

All notable changes to the Eddy core are recorded here: the engine the apps link, `eddy-server` and `eddy-cli`. The apps have their own changelogs ([Mac](apple/CHANGELOG.md)).

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the core follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Its version is `[workspace.package] version` in `Cargo.toml`; releases are tagged `core-vX.Y.Z`. A change to the sync protocol version is called out, because that, not the version number, decides which servers and devices can sync ([ADR 0009](docs/adr/0009-versions-and-compatibility.md)).

## [Unreleased]

Sync protocol 2. Servers and devices on protocol 1 must be updated together.

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
- Engine: `mark_read` marks items read and finished by hand, leaving the reading position; it is what the apps' Mark as Read does. Opening an item still only marks it read. A Continue Reading query lists the items named in `keep` even once they are finished.

### Changed

- Sync protocol 2: clients send their protocol version and servers report the oldest they accept; either side tells the other to update when they are too far apart (HTTP 426 from the server).
- A change the server cannot use (unknown kind, wrong value, a clock far ahead) is refused on its own; the rest of the upload goes through.
- Pairing keeps the FreshRSS account on the device and refuses a server that manages a different account.
- The server refreshes FreshRSS right after starting instead of one interval later.
- Data written by a newer version is refused instead of being opened.
- Disconnecting from a server asks it to forget the device, so the device no longer stays listed with a token that still works.
- A first sync is several times faster: pages are larger and compressed, each is stored in one step, and the next downloads while the current one is applied. Pulled pages now say how many changes the server has; this needs no protocol change, and older servers and devices ignore it.
- After a long sync the articles fetched for offline reading are the newest, several at a time.

### Fixed

- A read/star change that loses to a newer one is no longer pushed to FreshRSS, where it used to come back as the newest state.
- Removing a rule pack on one device removes it on the others.
- Revoking a device closes its open live connection.
- The server stops cleanly on SIGTERM.
- Articles stored but not published when the server stopped are ingested again.
- Live sync no longer stacks up connections, times out and pings, and uploads waiting changes when it reconnects.
- Images on hosts that refuse requests without a `Referer` (sspai's among them) load: a refused image is asked for once more the way a browser showing the article would.

[Unreleased]: https://github.com/WheheoHu/eddy/commits/dev
