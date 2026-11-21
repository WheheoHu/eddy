# eddy, for agents

`eddy` works on the user's Eddy library: the articles from their subscriptions,
the pages, books and PDFs they saved, and where they are in each. It talks to
the Eddy app, starting it in the background when it is not running, so what you
change shows up in the app at once.

## Ground rules

- Add `--json` to every command. The result is one JSON object on standard
  output; without it you get text meant for a person.
- A failed command exits non-zero and, with `--json`, prints
  `{"error": {"code": "...", "message": "..."}}`. Act on `code`; the message
  may be in Chinese. Codes: `not_found`, `invalid`, `no_content`, `offline`,
  `network`, `not_configured`, `unauthorized`, `app_unreachable`,
  `version_mismatch`, `store`, `internal`.
- Exit status: 0 done, 1 failed, 2 the command was not understood, 3 Eddy
  could not be reached.
- Items, feeds and series are named by ID (copy them from a listing). Tags and
  folders are named by name.
- Times are milliseconds since 1970. Pages count from 1. `progress` is 0 to 1.
- `items delete` and `series delete` remove things from every device and
  cannot be undone. Ask the user first. `items read-all` takes `--dry-run`.
- `summary --generate` spends the user's AI quota. Everything else is free.
- Progress of long commands goes to standard error; `--quiet` silences it.

## Finding things

```sh
eddy status --json                         # how Eddy is set up, sync state
eddy items --json --unread --limit 20      # newest unread articles and saved pages
eddy items --json --saved                  # what the user saved by hand
eddy items --json --continue               # started, not finished
eddy items --json --feed ID                # one subscription
eddy items --json --folder Tech --unread   # a folder of subscriptions
eddy items --json --tag later              # by tag
eddy items --json --title "path tracing"   # title contains
eddy items get ID [ID…] --json             # everything about items, with tags
eddy search "query" --json --limit 10      # full text; hits carry a snippet
eddy feeds --json                          # subscriptions: id, title, folder, unread
eddy folders --json
eddy tags --json
eddy collection --json                     # books, PDFs, saved pages, series
eddy series --json                         # books and web series
eddy series get ID --json                  # with its chapters, which are items
```

A list answers `{"items": [...], "total": N, "limit": L, "offset": O}`; page
with `--offset`. Only one of `--starred --saved --continue --feed --folder
--series --tag --title` at a time; `--unread` combines with any.

## Reading

```sh
eddy doc ID                      # the text as Markdown (title on top)
eddy doc ID --format text        # plain text
eddy doc ID --format blocks      # one line per block: block ID, kind, text
eddy doc ID --format json        # the document model
eddy doc ID --pages 3-7          # part of a PDF
eddy doc ID --json               # {"id","title","url","pages","text_len","content"}
eddy summary ID --json           # the summary Eddy already has, if any
eddy translation ID --json       # the stored translation, by block ID
```

Formulas are TeX between `$…$` and `$$…$$`. A book is a series: read its
chapters one by one. `no_content` means the text is not on this device;
`eddy items fetch ID` fetches an article's full text from its page.

## Adding

```sh
eddy add url https://example.com/article --json
eddy add file ~/Downloads/paper.pdf --json          # EPUB or PDF
eddy add text notes.md --title "Reading notes" --json
printf '%s' "$MARKDOWN" | eddy add text --title "Summary of the week" --json
eddy add text --format html page.html --json
```

`add text` saves what you wrote as a document of its own, in Saved. Without
`--title` the first `# heading` names it. `--url` says where the text came
from, so relative links resolve; it does not make it "the saved copy" of that
page. Each answers `{"id", "title", "existing"}`; a book also lists its
chapters.

## Changing

```sh
eddy items read ID [ID…]          # also: unread, star, unstar, finish
eddy items tag ID later physics   # tags are made as needed; untag removes
eddy items read-all --folder News --dry-run --json   # then without --dry-run
eddy items delete ID              # saved pages, PDFs, series: not undoable
eddy tags add NAME | rename OLD NEW | remove NAME
eddy feeds add URL --folder Tech  # a feed, or a page that names one
eddy feeds remove ID              # the feed leaves with its articles
eddy series create URL            # collect a table of contents into a series
eddy export epub ID -o book.epub
eddy sync                         # fetch new articles now
```

## Reading position

```sh
eddy progress get ID --json                 # progress, finished, block, offset, page, quote
eddy progress set ID --block BLOCK_ID       # block IDs: eddy doc ID --format blocks
eddy progress set ID --page 12              # PDFs
eddy progress set ID --end                  # read to the end
```

## Not here

Accounts, pairing with a server and signing out are done in the app's
Settings. Renaming or moving subscriptions and highlights are not available
yet.
