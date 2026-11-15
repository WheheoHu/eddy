# Changelog: core

All notable changes to the Eddy core are recorded here: the engine the apps link, `eddy-server` and `eddy-cli`. The apps have their own changelogs ([Mac](apple/CHANGELOG.md)).

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the core follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Its version is `[workspace.package] version` in `Cargo.toml`; releases are tagged `core-vX.Y.Z`. A change to the sync protocol version is called out, because that, not the version number, decides which servers and devices can sync ([ADR 0009](docs/adr/0009-versions-and-compatibility.md)).

## [Unreleased]

Sync protocol 2. Servers and devices on protocol 1 must be updated together.

### Added

- Server: admin page at `/admin`, on when `EDDY_ADMIN_PASSWORD` is set: pairing codes, devices, FreshRSS status and account, video jobs, service status.
- Server: `eddy-server --version`; `/v1/info` reports the version; `/v1/health` checks the database.
- Engine: works with the server offline. Changes queue and upload on their own, search and video job lists answer locally at once, and refresh fetches new articles from FreshRSS directly when the device has the account.
- Engine: status reports whether the server is reachable and how many changes are waiting or were refused.
- Engine: pairing with a server the device has not synced with before uploads its whole history.
- `eddy-cli`: `status`, `disconnect`, `freshrss`, `server-account`, `tag`, `tags`, `star`, `--version`.

### Changed

- Sync protocol 2: clients send their protocol version and servers report the oldest they accept; either side tells the other to update when they are too far apart (HTTP 426 from the server).
- A change the server cannot use (unknown kind, wrong value, a clock far ahead) is refused on its own; the rest of the upload goes through.
- Pairing keeps the FreshRSS account on the device and refuses a server that manages a different account.
- The server refreshes FreshRSS right after starting instead of one interval later.
- Data written by a newer version is refused instead of being opened.

### Fixed

- A read/star change that loses to a newer one is no longer pushed to FreshRSS, where it used to come back as the newest state.
- Removing a rule pack on one device removes it on the others.
- Revoking a device closes its open live connection.
- The server stops cleanly on SIGTERM.
- Articles stored but not published when the server stopped are ingested again.
- Live sync no longer stacks up connections, times out and pings, and uploads waiting changes when it reconnects.

[Unreleased]: https://github.com/WheheoHu/eddy/commits/dev
