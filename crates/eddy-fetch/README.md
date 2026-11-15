# eddy-fetch

A generic page-fetching pipeline and the rule pack format. This repository contains only the engine and the format description, and no site rules. Rule packs are maintained by the user and live on their own machine in `~/Library/Application Support/Eddy/rulepacks/`, or are distributed by their own eddy-server (see `docs/adr/0004`).

## Fetch strategies

For a given URL the pipeline makes a direct request first. When the result is not accepted (for example, the extracted body is too short), it tries the strategies listed in the rule's `strategies`, in order:

| Name | What it does |
| --- | --- |
| `headers` | Requests again with the rule's `user_agent`, `referer` and `headers` |
| `cookies` | Sends your own cookies from the rule (for example, the login session of a site you subscribe to) |
| `archive` | Asks the Internet Archive for a snapshot and takes the original page from it |
| `webview` | Hands the URL to the platform's WebView and takes the rendered page (provided by the app) |

## Rule pack format (TOML)

```toml
name = "My rules"
version = 1

[[site]]
# A leading dot matches every subdomain
domains = ["example.com", ".example.org"]
strategies = ["headers", "webview"]
user_agent = "Mozilla/5.0 ..."
referer = "https://example.com/"
headers = { "Accept-Language" = "en-US" }
cookies = { "session" = "your own session value" }

# Body extraction overrides: CSS selectors, or common XPath forms
body = ["article .post-content"]
strip = [".newsletter-signup", "//aside"]
next_page = "a.next"
```

Site files in the Full-Text RSS format (ftr-site-config) can also be dropped into `rulepacks/ftr/`, named after the domain, for example `example.com.txt`. Supported directives: `body`, `strip`, `strip_id_or_class`, `title`, `next_page_link`, `single_page_link`, `http_header(user-agent)`, `http_header(referer)`, `http_header(cookie)`. Only XPath expressions that translate directly into a CSS selector are supported; other entries are skipped.
