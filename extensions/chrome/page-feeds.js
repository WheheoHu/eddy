// Runs in the page: by the content script as a page loads, for the number
// on the toolbar icon, and from the worker when the popup opens on a page
// that was there before the extension was.

// What a page says of its own feeds, run in the page. Besides the links in
// its head that name a feed, the links in its text that look like one, and
// the page itself when it is a feed opened in a tab.
function feedsOnPage() {
  const feeds = [];
  const kinds = /(rss|atom|rdf)\+xml|feed\+json|^application\/json\+feed/i;
  for (const link of document.querySelectorAll('link[rel~="alternate" i][href], link[rel~="feed" i][href]')) {
    if (kinds.test(link.type) || /\bfeed\b/i.test(link.rel)) feeds.push({ url: link.href, title: link.title || "" });
  }
  // A feed shown as a page: Chrome wraps the XML, or shows it as text.
  const root = document.querySelector("rss, feed, rdf\\:RDF") || document.getElementById("webkit-xml-viewer-source-xml")?.firstElementChild;
  if (/(rss|atom)\+xml/i.test(document.contentType) || (root && /^(rss|feed|rdf:RDF)$/i.test(root.nodeName))) {
    feeds.push({ url: location.href, title: document.title || "" });
  }
  // Links a reader would take for a feed: by where they point, or by what
  // they say. Only on this site, and only a few.
  const looks = /(^|[\/.=_-])(rss|atom|feed)s?([\/.]?(xml|rss|atom|json))?\/?$|\/(rss|atom|feed)\.(xml|php|aspx?)$|\/index\.xml$/i;
  let linked = 0;
  for (const anchor of document.querySelectorAll("a[href]")) {
    if (linked >= 4) break;
    let url;
    try {
      url = new URL(anchor.href);
    } catch (e) {
      continue;
    }
    if (!/^https?:$/.test(url.protocol)) continue;
    const text = (anchor.textContent || anchor.title || anchor.getAttribute("aria-label") || "").trim();
    const same = url.hostname.split(".").slice(-2).join(".") === location.hostname.split(".").slice(-2).join(".");
    // What a page calls such a link, in English or in Chinese.
    const says = /^(rss|atom|rss ?订阅|订阅 ?rss|rss feed|feed)$/i.test(text);
    if ((same && looks.test(url.pathname + url.search)) || says) {
      feeds.push({ url: url.href, title: says || !text || text.length > 40 ? "" : text });
      linked++;
    }
  }
  return feeds;
}
