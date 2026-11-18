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
- Three-Pane fits the window's width, and the window can be as narrow as 480 points. As it narrows, the sidebar goes first: the back button over the list, or a swipe to the right on the list, shows it in the list's place. Narrower, the article has the whole window, and a swipe to the right on it slides the sidebar and list in over it; a click on the article, Esc or a swipe to the left sends them away. The narrowest window shows one column at a time, and a swipe to the right goes back: article, list, sidebar. The columns move with the fingers and settle where the swipe was heading once it has gone far enough, or go back where they were. The first button of the toolbar does the same without a trackpad. Where the sidebar takes the list's place or the whole window, it is set in larger type under bolder section titles.
- In the narrowest Three-Pane window, a book's contents open over the text from the Contents button and close when a chapter is picked.

### Changed

- A new look throughout: flat surfaces in two tones, square corners, large titles. The window has no title bar or toolbar; each column carries its own controls along its top, and the window's buttons sit in the first column.
- The mode switch (Three-Pane, Library, Gallery) is at the bottom of the sidebar, with video conversions beside it.
- Three-Pane: the list's name and unread count head the list in large type. Under the list a switch shows starred articles, unread ones, or all; it replaces the Unread Only button. Mark All as Read and search are over the list; Refresh and Add are over the sidebar.
- Three-Pane, in a window with room for the list and the article: the sidebar comes in beside the list, over the article, instead of taking the list's place. A swipe to the right on the article brings it in too; a click on the article, Esc, or choosing something sends it away.
- Library and Gallery fit a narrow window too, down to 480 points wide. Under 940 points the sidebar tucks away: the button at the start of the row of controls, or a swipe to the right, brings it in over the page, which sinks back behind it. A swipe to the left, a click on the page or Esc sends it away. A book's contents do the same while reading, and a swipe to the right on the text brings them in.
- When a window is resized past a point where a column tucks away or comes back, the columns glide to their new places.
- Choosing something in a sidebar that is in over the page leaves the sidebar in, and so does switching between Three-Pane, Library and Gallery from the switch at its foot. Picking a chapter from a book's contents still closes them.
- Three-Pane in its narrowest window: the sidebar is as wide as everywhere else and comes in beside the list, which moves along and sinks back, instead of taking the whole window. The button over the list shows it.
- The Library's covers are smaller in a narrow window, so a screen of the shelf still holds about a dozen books.
- While reading in the Library, the contents column has the mode switch at its foot.
- An article the columns have come in over sinks back behind them: a little smaller, with rounded corners, and dimmed.
- The reader's controls are over the article, with a button that closes it. Where they don't all fit, translation, summary, text style, full text and export move into a menu.
- Library and Gallery: the name of what is shown and how many items heads the page. The Library's filter field, sort order and covers-or-list switch are over the shelf.
- Settings open as a panel in the main window (⌘, or the command palette), with the window dimmed behind it; Esc or a click outside closes it. In a narrow window its categories become a row of icons down the side.
- Settings pages have a new look: a large title, bold section headings over ruled rows, square switches, and a segmented switch wherever there are only a few choices (Open in, Content appearance, Protocol, Keep up to, a full-text rule's mode).
- Articles and the Gallery glide further after a flick on the trackpad, and spring back when the glide reaches an end. Touching the trackpad stops it.
- Disconnect from Server also removes this Mac from the server's devices.
- Pairing with a sync server finishes as soon as this Mac is connected. The first sync runs in the background while the app stays usable, and articles appear as they arrive.
- An unreachable server is a quiet status, not an error banner. Refresh says when new articles came from FreshRSS directly; search says when it only covered this Mac.
- A data directory chosen with `EDDY_DATA_DIR` keeps its secrets in its own keychain item, apart from the real installation's.
- Open in Browser, in the reader's toolbar and the media viewer, shows the mark of your default browser: Chrome, Safari, Firefox, Edge, Arc, Opera or Vivaldi, and a globe for any other.
- Three-Pane's list buttons (Unread Only, Mark All as Read, Refresh, Sort By) sit at the start of the toolbar, over the list. The list keeps a width of its own, 360 points until the line beside it is dragged; the width it had before isn't carried over.
- In a Three-Pane window too narrow for the list, a swipe to the right on a book's chapter shows the list instead of the previous chapter. Pull down past the top of the chapter or press ⌘← for the previous one.

### Removed

- The Home button while reading in the Library: the arrow at the start of the row of controls goes back to the shelf.
- The Share button in the reader's toolbar. Copy Link is still in the article list's context menu and the command palette.

### Fixed

- Swiping columns in and back is smooth: they no longer stutter while following the fingers, or hitch as the swipe ends.
- Scrolling an article no longer stops when the pointer reaches a table. A table wider than the text still scrolls sideways.
- A table on the first screen of an article shows when the article opens, not only after scrolling away and back.
- The Gallery scrolls without hitching: its list no longer reloads while the grid is moving, and thumbnails are decoded at the size they are shown.
- An article keeps its place when the window is resized: the line at the top stays at the top while the text wraps to the new width.

[Unreleased]: https://github.com/WheheoHu/eddy/commits/dev
