# Changelog: Eddy for Mac

All notable changes to the Mac app are recorded here. The core it is built on has its own changelog ([core](../CHANGELOG.md)); each release below names the core version it was built with.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the app follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Its version is `MARKETING_VERSION` in `apple/project.yml`; releases are tagged `mac-vX.Y.Z`.

## [Unreleased]

### Added

- Settings → Accounts shows whether the sync server is online, its version, and how many changes are waiting to upload.
- In server mode the FreshRSS section shows the server's account and lets this Mac keep it for refreshing while the server is offline; a server without an account can be given one here.
- Settings → About shows the app version and the core version.
- Lists reload when changes arrive from the server in the background.
- Settings → Accounts → Sign Out and Erase Data: removes accounts, library, downloaded articles, API keys and website sign-ins from this Mac and restarts Eddy as new. Changes not uploaded yet are sent first, and Eddy asks before losing any that cannot be. Appearance and layout settings are kept.
- A long sync shows its progress in Settings → Accounts and at the bottom of the sidebar, which also says when the sync server is offline.

### Changed

- Disconnect from Server also removes this Mac from the server's devices.
- Pairing with a sync server finishes as soon as this Mac is connected. The first sync runs in the background while the app stays usable, and articles appear as they arrive.
- An unreachable server is a quiet status, not an error banner. Refresh says when new articles came from FreshRSS directly; search says when it only covered this Mac.
- A data directory chosen with `EDDY_DATA_DIR` keeps its secrets in its own keychain item, apart from the real installation's.

[Unreleased]: https://github.com/WheheoHu/eddy/commits/dev
