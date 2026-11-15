# Changelog: core

The Rust core: the engine the apps link, `eddy-server` and `eddy-cli`. Its version is `[workspace.package] version` in `Cargo.toml`; releases are tagged `core-vX.Y.Z`. The apps have their own changelogs (`apple/CHANGELOG.md`). See `docs/adr/0009-versions-and-compatibility.md`.

## 0.2.0 (unreleased)

Sync protocol 2. Servers and devices on protocol 1 must be updated together.

### Server

- Admin page at `/admin`, on when `EDDY_ADMIN_PASSWORD` is set: pairing codes, devices, FreshRSS status and account, video jobs, service status.
- Revoking a device closes its open live connection.
- A read/star change that loses to a newer one is no longer pushed to FreshRSS, where it used to come back as the newest state.
- A change the server cannot use (unknown kind, wrong value, a clock far ahead) is refused on its own; the rest of the upload goes through.
- Clients with an incompatible protocol are told to update (HTTP 426).
- Removing a rule pack on one device removes it on the others.
- `eddy-server --version`; `/v1/info` reports the version; `/v1/health` checks the database.
- Stops cleanly on SIGTERM; refreshes FreshRSS right after starting.
- Articles stored but not published when the process stopped are ingested again.

### Engine

- Works with the server offline: changes queue and upload on their own, search and video job lists answer locally at once, and refresh fetches new articles from FreshRSS directly when the device has the account.
- Status reports whether the server is reachable and how many changes are waiting or were refused.
- Pairing keeps the FreshRSS account on the device, refuses a server that manages a different account, and uploads the whole history to a server the device has not synced with before.
- Live sync no longer stacks up connections, times out and pings, and uploads waiting changes when it reconnects.
- Data written by a newer version is refused instead of being opened.

### eddy-cli

- `status`, `disconnect`, `freshrss`, `server-account`, `tag`, `tags`, `star`, `--version`.

## 0.1.0

Everything before versions were tracked.
