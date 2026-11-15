# Changelog: Eddy for Mac

The Mac app. Its version is `MARKETING_VERSION` in `apple/project.yml`; releases are tagged `mac-vX.Y.Z`. Each entry names the core version it was built with; the core's own changes are in the root `CHANGELOG.md`.

## 0.2.0 (unreleased)

Built with core 0.2.0.

- Settings → Accounts shows whether the sync server is online, its version, and how many changes are waiting to upload.
- In server mode the FreshRSS section shows the server's account and lets this Mac keep it for refreshing while the server is offline; a server without an account can be given one here.
- An unreachable server is a quiet status, not an error banner. Refresh says when new articles came from FreshRSS directly; search says when it only covered this Mac.
- Lists reload when changes arrive from the server in the background.
- Settings → About shows the app version and the core version.
- A data directory chosen with `EDDY_DATA_DIR` keeps its secrets in its own keychain item, apart from the real installation's.

## 0.1.0

Everything before versions were tracked.
