// node extensions/tests/popup.test.js
// Runs the popup's script against a stand-in DOM and chrome API.
const fs = require("fs");
const src = fs.readFileSync(require("path").join(__dirname, "../chrome/popup.js"), "utf8");
function el(id) {
  return { id, hidden: id !== "", textContent: "", children: [], className: "", disabled: false, focused: false,
    append(...kids) { this.children.push(...kids); }, focus() { this.focused = true; } };
}
let last;
function run(page, replies) {
  const els = {}, sent = [];
  let closed = false;
  const document = { getElementById: (id) => (els[id] ??= el(id)), createElement: () => el("") };
  const chrome = { runtime: { openOptionsPage() { sent.push("options"); }, sendMessage(msg, cb) { sent.push(msg.type); last = msg; cb(msg.type === "inspect" ? page : replies[msg.type]); } } };
  const window = { close() { closed = true; } };
  new Function("document", "chrome", "window", "navigator", src)(document, chrome, window, {});
  return { els, sent, closed: () => closed };
}
const assert = require("assert");
// Opening asks what is here and saves nothing.
let p = run({ ok: true, url: "https://sspai.com/post/1", title: "A post", feeds: [{ url: "https://sspai.com/feed", title: "少数派" }, { url: "https://sspai.com/a.atom", title: "" }] }, { capture: { ok: true, url: "https://sspai.com/post/1" }, subscribe: { ok: true } });
assert.deepStrictEqual(p.sent, ["inspect"]);
assert.strictEqual(p.els.ask.hidden, false);
assert.strictEqual(p.els.feeds.hidden, false);
assert.strictEqual(p.els.save.focused, true);
assert.strictEqual(p.els.host.textContent, "sspai.com");
const rows = p.els.feeds.children;
assert.strictEqual(rows.length, 2);
assert.strictEqual(rows[0].children[0].textContent, "少数派");
assert.strictEqual(rows[1].children[0].textContent, "/a.atom");
// The save button saves, then offers Show.
p.els.save.onclick();
assert.deepStrictEqual(p.sent, ["inspect", "capture"]);
assert.strictEqual(p.els.status.textContent, "Saved to Eddy");
assert.strictEqual(p.els.ask.hidden, true);
assert.strictEqual(p.els.show.hidden, false);
// A feed's button hands it to Eddy and closes the popup; nothing is saved.
p = run({ ok: true, url: "https://sspai.com/", title: "少数派", feeds: [{ url: "https://sspai.com/feed", title: "少数派" }] }, { subscribe: { ok: true } });
p.els.feeds.children[0].children[1].onclick();
assert.deepStrictEqual(p.sent, ["inspect", "subscribe"]);
assert.strictEqual(p.closed(), true);
// Eddy not reachable: the popup says so.
p = run({ ok: true, url: "https://sspai.com/", title: "少数派", feeds: [{ url: "https://sspai.com/feed", title: "" }] }, { subscribe: { ok: false, error: "host missing" } });
p.els.feeds.children[0].children[1].onclick();
assert.strictEqual(p.els.status.textContent, "Couldn’t open Eddy");
assert.strictEqual(p.els.detail.textContent, "host missing");
// No feeds: the section stays away. A page that can't be saved says so.
p = run({ ok: true, url: "https://example.com/a", title: "Example", feeds: [] }, { subscribe: { ok: true } });
assert.strictEqual(p.els.feeds.hidden, false);
assert.strictEqual(p.els.feeds.children.length, 1);
assert.strictEqual(p.els.feeds.children[0].children[1].textContent, "Add Manually…");
p.els.feeds.children[0].children[1].onclick();
assert.deepStrictEqual(p.sent, ["inspect", "subscribe"]);
p = run({ ok: false, error: "Only web pages can be saved." }, {});
assert.strictEqual(p.els.status.textContent, "Can’t save this page");
assert.deepStrictEqual(p.sent, ["inspect"]);
// Feeds RSSHub makes for the page are listed under their own heading; with
// some of those and none of the page's own, there is no "add by hand" row.
p = run({ ok: true, url: "https://sspai.com/index", title: "少数派", feeds: [], radar: [{ url: "https://hub.example/sspai/index", title: "少数派 · 首页" }], radarLoaded: true }, { subscribe: { ok: true } });
assert.strictEqual(p.els.feeds.hidden, true);
assert.strictEqual(p.els.radar.hidden, false);
assert.strictEqual(p.els.radar.children[0].children[0].textContent, "少数派 · 首页");
p.els.radar.children[0].children[1].onclick();
assert.deepStrictEqual(last, { type: "subscribe", url: "https://hub.example/sspai/index", title: "少数派 · 首页" });
assert.strictEqual(p.els["radar-note"]?.textContent ?? "", "");
// Rules that never loaded are said to be missing, with the way to settings.
p = run({ ok: true, url: "https://example.com/a", title: "Example", feeds: [], radar: [], radarLoaded: false }, {});
assert.strictEqual(p.els.radar.hidden, true);
assert.strictEqual(p.els["radar-note"].textContent, "RSSHub rules not loaded");
p.els.options.onclick({ preventDefault() {} });
assert.deepStrictEqual(p.sent, ["inspect", "options"]);
console.log("popup logic ok");
