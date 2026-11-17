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
- Gallery: View → Zoom In (⌘+) and Zoom Out (⌘−) change the size of the thumbnails.
- Three-Pane fits the window's width, and the window can be as narrow as 480 points. As it narrows, the sidebar goes first: the back button over the list, or a swipe to the right on the list, shows it in the list's place. Narrower, the article has the whole window, and a swipe to the right on it slides the sidebar and list in over it; a click on the article, Esc or a swipe to the left sends them away. The narrowest window shows one column at a time, and a swipe to the right goes back: article, list, sidebar. The columns move with the fingers and settle where the swipe was heading once it has gone far enough, or go back where they were. The first button of the toolbar does the same without a trackpad.
- In the narrowest Three-Pane window, a book's contents open over the text from the Contents button and close when a chapter is picked.

### Changed

- Articles and the Gallery glide further after a flick on the trackpad, and spring back when the glide reaches an end. Touching the trackpad stops it.
- Disconnect from Server also removes this Mac from the server's devices.
- Pairing with a sync server finishes as soon as this Mac is connected. The first sync runs in the background while the app stays usable, and articles appear as they arrive.
- An unreachable server is a quiet status, not an error banner. Refresh says when new articles came from FreshRSS directly; search says when it only covered this Mac.
- A data directory chosen with `EDDY_DATA_DIR` keeps its secrets in its own keychain item, apart from the real installation's.
- Open in Browser, in the reader's toolbar and the media viewer, shows the mark of your default browser: Chrome, Safari, Firefox, Edge, Arc, Opera or Vivaldi, and a globe for any other.
- Three-Pane's list buttons (Unread Only, Mark All as Read, Refresh, Sort By) sit at the start of the toolbar, over the list. The list keeps a width of its own, 360 points until the line beside it is dragged; the width it had before isn't carried over.
- In a Three-Pane window too narrow for the list, a swipe to the right on a book's chapter shows the list instead of the previous chapter. Pull down past the top of the chapter or press ⌘← for the previous one.

### Removed

- The Share button in the reader's toolbar. Copy Link is still in the article list's context menu and the command palette.

### Fixed

- Scrolling an article no longer stops when the pointer reaches a table. A table wider than the text still scrolls sideways.
- A table on the first screen of an article shows when the article opens, not only after scrolling away and back.
- The Gallery scrolls without hitching: its list no longer reloads while the grid is moving, and thumbnails are decoded at the size they are shown.
- An article keeps its place when the window is resized: the line at the top stays at the top while the text wraps to the new width.

[Unreleased]: https://github.com/WheheoHu/eddy/commits/dev
