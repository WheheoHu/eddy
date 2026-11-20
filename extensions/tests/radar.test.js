const assert = require("assert");
// node extensions/tests/radar.test.js
const { matchSource, fillTarget, radarFeeds, countRules } = require("../chrome/radar.js");
const rules = {
  "sspai.com": {
    _name: "少数派",
    ".": [
      { title: "首页", source: ["/index"], target: "/sspai/index" },
      { title: "作者", source: ["/u/:id/posts", "/u/:id"], target: "/sspai/author/:id" },
      { title: "专栏", source: ["/column/:id"], target: "/sspai/column/:id" },
      { title: "标签订阅", source: ["/"], target: "/sspai/tag/:keyword" },
    ],
    shortcuts: [{ title: "Shortcuts Gallery", source: ["/*"], target: "/sspai/shortcuts" }],
  },
  "bilibili.com": {
    _name: "哔哩哔哩 bilibili",
    space: [{ title: "UP 主投稿", source: ["/:uid"], target: "/bilibili/user/video/:uid" }],
    www: [{ title: "视频评论", source: ["/video/:bvid"], target: "/bilibili/video/reply/:bvid" }],
  },
  "github.com": {
    _name: "GitHub",
    ".": [
      { title: "Repo Issues", source: ["/:user/:repo/issues", "/:user/:repo/issues/:id", "/:user/:repo"], target: "/github/issue/:user/:repo/:state?/:labels?" },
      { title: "Trending", source: ["/trending/:language?"], target: "/github/trending/daily/:language?" },
    ],
  },
  "youtube.com": {
    _name: "YouTube",
    www: [{ title: "Channel", source: ["/watch?v=:id"], target: "/youtube/video/:id" }, { title: "Digits", source: ["/c/:n{[0-9]+}"], target: "/youtube/c/:n" }],
  },
  "example.co.uk": { _name: "Example", ".": [{ title: "Example", source: ["/news/:slug+"], target: "/example/news/:slug+" }], broken: "not a list" },
};
const feeds = (page, base = "https://rsshub.example", key) => radarFeeds(rules, page, base, key).map((f) => f.url.replace("https://rsshub.example", ""));

// A page the rules describe, with and without www; the site-wide rule
// whose target wants a name the page doesn't give is left out.
assert.deepStrictEqual(feeds("https://sspai.com/index"), ["/sspai/index"]);
assert.deepStrictEqual(feeds("https://www.sspai.com/u/abc/posts?page=2"), ["/sspai/author/abc"]);
assert.deepStrictEqual(feeds("https://sspai.com/u/abc/"), ["/sspai/author/abc"]);
assert.deepStrictEqual(feeds("https://sspai.com/"), []);
assert.deepStrictEqual(feeds("https://sspai.com/post/118879"), []);
// Subdomains have their own lists; a deeper one does not borrow them.
assert.deepStrictEqual(feeds("https://shortcuts.sspai.com/anything/at/all"), ["/sspai/shortcuts"]);
assert.deepStrictEqual(feeds("https://space.bilibili.com/2267573"), ["/bilibili/user/video/2267573"]);
assert.deepStrictEqual(feeds("https://www.bilibili.com/video/BV1xx411c7mD/"), ["/bilibili/video/reply/BV1xx411c7mD"]);
assert.deepStrictEqual(feeds("https://bilibili.com/video/BV1xx411c7mD"), ["/bilibili/video/reply/BV1xx411c7mD"]);
assert.deepStrictEqual(feeds("https://m.bilibili.com/video/BV1xx411c7mD"), []);
// Optional names: dropped from the target when the page has none.
assert.deepStrictEqual(feeds("https://github.com/DIYgod/RSSHub/issues"), ["/github/issue/DIYgod/RSSHub"]);
assert.deepStrictEqual(feeds("https://github.com/trending"), ["/github/trending/daily"]);
assert.deepStrictEqual(feeds("https://github.com/trending/rust"), ["/github/issue/trending/rust", "/github/trending/daily/rust"]);
// A value from the query, an expression in braces, the rest of a path, a
// site under a two-part suffix.
assert.deepStrictEqual(feeds("https://www.youtube.com/watch?v=abc123&t=4"), ["/youtube/video/abc123"]);
assert.deepStrictEqual(feeds("https://www.youtube.com/watch"), []);
assert.deepStrictEqual(feeds("https://www.youtube.com/c/123"), ["/youtube/c/123"]);
assert.deepStrictEqual(feeds("https://www.youtube.com/c/abc"), []);
assert.deepStrictEqual(feeds("https://example.co.uk/news/2026/11/a-story"), ["/example/news/2026/11/a-story"]);
// Names with characters that mean something in an address are escaped.
assert.deepStrictEqual(feeds("https://sspai.com/column/a%20b%2Fc"), ["/sspai/column/a%20b%2Fc"]);
// The instance's address, with or without a slash, and its access key.
assert.strictEqual(radarFeeds(rules, "https://sspai.com/index", "https://hub.example/", "k y")[0].url, "https://hub.example/sspai/index?key=k%20y");
assert.strictEqual(radarFeeds(rules, "https://sspai.com/index", "https://hub.example")[0].title, "少数派 · 首页");
// Nothing to go on: no rules, a page that is not on the web, junk.
assert.deepStrictEqual(radarFeeds(null, "https://sspai.com/index", "https://hub.example"), []);
assert.deepStrictEqual(radarFeeds(rules, "chrome://extensions", "https://hub.example"), []);
assert.deepStrictEqual(radarFeeds(rules, "not an address", "https://hub.example"), []);
assert.deepStrictEqual(radarFeeds({ "sspai.com": { ".": [{ source: "x" }, null, { source: ["/(("], target: "/a" }] } }, "https://sspai.com/index", "h"), []);
assert.strictEqual(countRules(rules), 12);
assert.strictEqual(fillTarget("/a/:b", {}), null);
assert.deepStrictEqual(matchSource("/a/:b?", new URL("https://x.y/a")), {});
console.log("radar ok");
