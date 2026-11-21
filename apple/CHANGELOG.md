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
- Opening an article or chapter brings its text in with a short rise and fade. Going on to another, the text comes up from below when the new one lies after the one before (lower in the list, or a later chapter) and settles from above when it lies before it. With Reduce Motion on it only fades.
- Continue Reading can show everything, only subscription articles, or only what is in the collection, with a switch under the list in Three-Pane and under the shelf in the Library. The Library's Continue Reading also shelves the subscriptions that have part-read articles.
- Mark as Read in more places: a button beside the star over the article, Reader → Mark as Read (⇧⌘U) and Mark All as Read… (⇧⌘K) in the menu bar, Mark All as Read… on a subscription or folder in the sidebar and on a subscription on the Library's shelf, and Mark as Read on books, chapters, saved pages and PDFs.
- Continue Reading: an article read to its end shows a check where its progress was, and stays listed until the list is opened again.
- The row the keys are at carries a bar in the accent colour at its start: the folder or subscription chosen while the keys are in the sidebar, the article or entry while they are in the list or the text, the chapter in a book’s contents. It follows clicks as well as keys.
- Three-Pane by keyboard: Right goes on a column and Left back one, as Return and Esc do. Return or Right in the sidebar goes to the list and opens its first row, unless one of its rows is open already. Left on a subscription goes to its folder. Space folds the folder chosen, and unfolds it.
- Gallery by keyboard: Up and Down choose in the sidebar, Return or Right goes to the pictures, the arrows move among them, Space or Return opens the one framed and Esc closes it; Esc again, or Left from the start of a row, goes back to the sidebar.
- Switching between Three-Pane, Library and Gallery puts the keys in the first column, at what is chosen there; in the Library with a book carried over from another mode, in its contents.
- Settings → Shortcuts lists the keys the app answers to. They can’t be changed yet.
- View → Show Contents / Hide Contents (⌘B) shows or hides a book’s contents beside the text.
- Shift with Left or Right turns to the previous or next chapter or article while reading, as a swipe that way does in a book. ⌘← and ⌘→ still do.
- Three-Pane: Space on a subscription folds the folder it is in. Going from the sidebar to the list with Return or Right scrolls the list to the article open, if it had been scrolled away.
- Add Feed… in the Add menu (and under File): subscribes to a feed from its address, or from a website's address when the site names its feed, into a folder chosen from the ones there are, a new one, or none. FreshRSS does the subscribing, through the sync server when Eddy is paired with one; while the server is offline the sheet says so. What FreshRSS answers (not a feed, subscribed already) is shown beside the address. The new subscription is listed at once and its articles follow.
- Chrome extension 0.2.0: the popup lists the feeds the page names, each with a button that opens Add Feed in Eddy on that feed; a page that names none offers its own address to start from. `eddy://add-feed?url=…` opens the same sheet.
- Subscriptions show their site's icon, in colour: in the sidebars of Three-Pane and the Gallery in place of the feed symbol, before the feed's name on each article in a list, and beside the title of a subscription opened in the Library. A subscription whose site has no icon keeps the symbol. Icons drawn in black or white alone are set on a small plate of the other tone where they would vanish into the background. They are fetched once from the sites themselves and then shown offline.
- Settings → Storage → Clear Cache removes what the offline cache holds beyond what is always kept (unread, starred, saved and in-progress articles), and says how much it freed. A large cache takes a little while; Eddy stays usable meanwhile.
- Unsubscribe… on a subscription, in Three-Pane's sidebar and on the Library's shelf. After a question, FreshRSS unsubscribes (through the sync server when Eddy is paired with one; not while it is offline) and the feed leaves with its articles, starred ones included. Pages saved from it stay.
- Library: a subscription's cover is made from its site's icon. The icon sits near its own size at the top, not blown up, on a soft field of its own colours, with the name below in dark or light type as the field asks. Subscriptions whose site has no icon keep the cover they had. In the list layout a subscription's row shows its icon too.
- Chrome extension 0.3.0: it has Eddy's icon in the toolbar. It finds more feeds: those RSSHub can make for the page (by the rules RSSHub Radar uses, fetched daily from the RSSHub instance set in the extension's options, `rsshub.app` to begin with), links in a page's text that point to a feed, and a feed opened in a tab. Chrome asks to allow the new version, which may read `rsshub.app`; an instance of your own is allowed when you enter it.
- Chrome extension 0.5.0: the toolbar icon carries the number of feeds the page in a tab has, its own and RSSHub's together, and its tooltip says so. To count them the extension now runs in every page you open and looks for feed links there, so Chrome lists it as able to read all sites; nothing it finds leaves the browser. The number can be switched off in the extension's settings.

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
- Mark as Read, chosen by hand, also finishes the item, so it leaves Continue Reading. An item that is part read offers both Mark as Read and Mark as Unread. Opening an article still only marks it read.
- Moving to the next or previous article or chapter fades the text in from the side it lies on, like every other way of opening one, instead of sliding the whole reader.
- The row of the article being read shows its progress as reading goes on, not only after leaving it.
- The sidebars of Three-Pane and the Library follow each other. A folder, a kind of the collection, Saved or Continue Reading chosen in one is what the other shows when switched to; a subscription chosen in Three-Pane is the one picked on the Library's shelf, and a subscription opened in the Library is the list Three-Pane shows. Unread, Starred and tags, which the Library has no shelf for, leave its shelf as it was.
- Three-Pane keeps what its sidebar had chosen when coming back from another mode, instead of starting at All Articles, and scrolls the sidebar to it.
- Going back to the Library's shelf closes the book: switching mode after that no longer brings its last article back.
- Three-Pane: a folder’s arrow stands in its row beside the folder icon, not at the sidebar’s edge, and a folder folds with the same slide whether its arrow is clicked or Space pressed. Left and Right no longer fold folders.
- Three-Pane: Left in an article goes back to the list instead of up a page; Up still scrolls. The Library’s reader keeps Left and Right for paging.
- While reading, Up and Down move a page at a time, not a line.
- Three-Pane: while the keys are in the article, the bar that marks its row in the list is at the row’s end, on the article’s side.
- The media viewer: Up and Down slide the next item in from below or above, as a swipe does, instead of swapping it in place.
- A page turned with Up or Down keeps a fifth of what was showing in view, to go on reading from.
- A book’s contents, and a subscription’s articles beside the text in the Library, mark the row open the way Three-Pane’s list does: the bar at the row’s start while the keys are in the contents, at its end while they are in the text. In Three-Pane with a book’s contents showing, the chapter carries the bar while reading, not the book in the list.
- Three-Pane: Left in the sidebar no longer sends it away where it lies over the other columns; Esc does.
- Reading: Right, held, does what a swipe to the left does and acts when let go: an article’s web page, or the next chapter of a book. Left held in the web page goes back to the text; in the Library’s reader it turns to the previous chapter. Neither pages any more; Up and Down do.
- The media viewer: in an item with several pictures, Left and Right and a sideways swipe slide the next picture in, and a swipe follows the fingers and can be held part-way, as between items.
- The media viewer: Left and Right on a single picture no longer go to another item; Up and Down do.
- The media viewer works out the items before and after ahead of time and has their pictures decoded, so the one slid to is there at once; a video shows its poster until its first frame.
- View → Show or Hide Contents (⌘B) replaces Show Contents / Hide Contents.
- Library and Gallery: Esc on the shelf or among the pictures brings the sidebar in where it was away (hidden, or off the edge of a narrow window), with the keys in it. Return or Right from a sidebar that lies over the shelf or the pictures sends it away again. Switching to either mode puts the keys in the sidebar only if it shows.
- Chrome extension: the toolbar button no longer saves the page as soon as its popup opens. The popup shows the page's title and waits for Save (Return presses it), so ⌘⇧E followed by Return saves. The right-click menu's items still save at once. Reload the extension in `chrome://extensions` after updating.
- Subscriptions that FreshRSS keeps in its default category are listed outside any folder, instead of in a folder named "Uncategorized".
- Settings → Translation, Summaries and Video: the language is picked from a menu, Chinese (Simplified) or English for now, instead of typed. Video's can be left to follow Summaries. A language typed in earlier stays chosen until another is picked.
- A feed that is removed in FreshRSS itself now takes its articles out of the lists too, instead of leaving them behind as read articles of no feed.
- Chrome extension 0.4.0: its words are in English, like the app's. Its settings open as a page in a tab of their own, with a Test button that asks the RSSHub instance typed in for its rules and says how many came back, or what went wrong, without saving anything; Save and Update Rules are separate buttons.

### Removed

- The Home button while reading in the Library: the arrow at the start of the row of controls goes back to the shelf.
- The Share button in the reader's toolbar. Copy Link is still in the article list's context menu and the command palette.

### Fixed

- Library and Gallery in a narrow window: Esc could leave the keys in a sidebar that was out of sight, and nothing brought it in.
- Three-Pane, in a window where the sidebar and list come in over the article: Left in the list sent them away instead of going on to the sidebar.
- ⌘B did nothing where a book’s contents lie over the text instead of beside it: Three-Pane in its narrowest window, and the Library under 940 points.
- Three-Pane, in a window with room for the list and the article: Right from the sidebar and Left from the list only showed and hid the sidebar. The keys now go to the list with the sidebar left in place, and back to it.
- Three-Pane, in a window too narrow to show the sidebar beside the other columns: the keys could start in the sidebar while it was out of sight.
- The media viewer: after a few items, a swipe up or down on the trackpad slid towards the next item and then came back to the one it started from.
- Three-Pane: rows of the sidebar could stand at two different distances from its edge.
- Switching from the Library to Three-Pane closed the article or chapter being read.
- Three-Pane: a subscription folder in the sidebar didn’t fold or unfold when its arrow was clicked or Left or Right pressed, until something else was chosen.
- An article without pictures opened at its top instead of where it was left, and the top was then saved as its position, so the place was lost; it also dropped out of Continue Reading.
- Opening an article again saved a position a paragraph further back each time.
- After Mark as Unread on the article being read, reading on is saved again when leaving it. Leaving without reading on still leaves it unread and unstarted.
- Continue Reading in Three-Pane: pulling past the end or the top of an article said “This is the last article” or “This is the first article” instead of moving on. Reading an article moved it to the top of the list, and reaching its end took it out; the rows now stay where they were while the list is on screen.
- A sideways swipe over the sidebar or a list is picked up when the fingers don't travel perfectly level, and no longer nudges the list up or down before the columns move.
- Swiping columns in and back is smooth: they no longer stutter while following the fingers, or hitch as the swipe ends.
- Scrolling an article no longer stops when the pointer reaches a table. A table wider than the text still scrolls sideways.
- A table on the first screen of an article shows when the article opens, not only after scrolling away and back.
- The Gallery scrolls without hitching: its list no longer reloads while the grid is moving, and thumbnails are decoded at the size they are shown.
- An article keeps its place when the window is resized: the line at the top stays at the top while the text wraps to the new width.
- Articles from sites that guard their pictures against hotlinking (少数派 among them) show their images in Reader. A picture that can't be loaded is noted in the log.
- Opening an unread article and turning to its web page straight away no longer stops in the middle of the slide. The lists used to be counted again on the main thread a moment after an article was marked read, a fifth of a second in a large library; they are now read in the background and redrawn once the slide is over. The first web page of a session no longer starts with a pause either.
- Settings: the line between the categories and the page no longer shows the window behind the panel through it.

[Unreleased]: https://github.com/WheheoHu/eddy/commits/dev
