// node extensions/tests/badge.test.js
// The number on the toolbar icon: the content script's report of a page's
// feeds, and what the worker makes of it.
const fs = require("fs");
const path = require("path");
const assert = require("assert");
const radar = require("../chrome/radar.js");
const read = (name) => fs.readFileSync(path.join(__dirname, "../chrome", name), "utf8");
const settle = () => new Promise((done) => setTimeout(done, 5));

const RULES = { "sspai.com": { _name: "少数派", ".": [{ title: "首页", source: ["/index"], target: "/sspai/index" }] } };

// The worker, with stand-ins for the browser.
function worker(stored = {}) {
  const kept = { rsshub: "https://hub.example", key: "", rules: RULES, rulesAt: Date.now(), rulesFrom: "https://hub.example", rulesError: "", ...stored };
  const badges = {}, titles = {}, scans = [], listeners = {};
  const on = (name) => ({ addListener: (fn) => { listeners[name] = fn; } });
  const chrome = {
    storage: { local: { get: async (d) => ({ ...d, ...kept }), set: async (v) => { Object.assign(kept, v); } }, onChanged: on("changed") },
    action: {
      setBadgeBackgroundColor: async () => {},
      setBadgeTextColor: async () => {},
      setBadgeText: async ({ tabId, text }) => { if (tabId === 99) throw new Error("no tab"); badges[tabId] = text; },
      setTitle: async ({ tabId, title }) => { titles[tabId] = title; },
    },
    runtime: { onMessage: on("message"), onInstalled: on("installed"), sendNativeMessage(_h, _m, reply) { reply({ ok: true }); }, lastError: null },
    contextMenus: { create() {}, onClicked: on("menu") },
    tabs: {
      query: async () => [{ id: 1 }, { id: 2 }],
      sendMessage: async (tabId, message) => { scans.push([tabId, message.type]); if (tabId === 2) throw new Error("no script there"); },
    },
    scripting: {},
  };
  new Function("chrome", "importScripts", "radarFeeds", "countRules", "fetchRules", "feedsOnPage", "setTimeout", read("background.js"))(
    chrome, () => {}, radar.radarFeeds, radar.countRules, async () => RULES, () => [], (fn) => fn());
  return { chrome, kept, badges, titles, scans, listeners };
}

(async () => {
  // A page's own feeds and RSSHub's are counted together, each once.
  let w = worker();
  w.listeners.message({ type: "feeds", url: "https://sspai.com/index", feeds: ["https://sspai.com/feed", "https://sspai.com/a.atom"] }, { tab: { id: 7 } }, () => {});
  await settle();
  assert.strictEqual(w.badges[7], "3");
  assert.strictEqual(w.titles[7], "Save to Eddy · 3 feeds on this page");
  w.listeners.message({ type: "feeds", url: "https://sspai.com/index", feeds: ["https://hub.example/sspai/index"] }, { tab: { id: 7 } }, () => {});
  await settle();
  assert.strictEqual(w.badges[7], "1");
  assert.strictEqual(w.titles[7], "Save to Eddy · 1 feed on this page");
  // RSSHub alone; then a page with nothing: no number at all.
  w.listeners.message({ type: "feeds", url: "https://sspai.com/index", feeds: [] }, { tab: { id: 8 } }, () => {});
  w.listeners.message({ type: "feeds", url: "https://example.com/", feeds: [] }, { tab: { id: 9 } }, () => {});
  await settle();
  assert.deepStrictEqual([w.badges[8], w.badges[9], w.titles[9]], ["1", "", "Save to Eddy"]);
  // A tab that closed meanwhile, a message from no tab, junk for feeds.
  w.listeners.message({ type: "feeds", url: "https://sspai.com/index", feeds: [] }, { tab: { id: 99 } }, () => {});
  w.listeners.message({ type: "feeds", url: "https://sspai.com/index", feeds: [] }, {}, () => {});
  w.listeners.message({ type: "feeds", url: "https://sspai.com/index", feeds: "x" }, { tab: { id: 10 } }, () => {});
  await settle();
  assert.strictEqual(w.badges[10], "1");

  // Switched off in the settings: no numbers, whatever a page has.
  w = worker({ badge: false });
  w.listeners.message({ type: "feeds", url: "https://sspai.com/index", feeds: ["https://sspai.com/feed"] }, { tab: { id: 7 } }, () => {});
  await settle();
  assert.strictEqual(w.badges[7], "");
  // Switching it asks every tab to count again; a tab with no script to
  // ask loses its number.
  w.badges[2] = "4";
  w.listeners.changed({ badge: { newValue: true } }, "local");
  await settle();
  assert.deepStrictEqual(w.scans, [[1, "scan"], [2, "scan"]]);
  assert.strictEqual(w.badges[2], "");
  w.listeners.changed({ rules: {} }, "local");
  w.listeners.changed({ badge: {} }, "sync");
  await settle();
  assert.strictEqual(w.scans.length, 2);

  // Saving from the right-click menu shows a tick, then the count again.
  w = worker();
  await w.listeners.menu({ menuItemId: "eddy-save-link", linkUrl: "https://example.com/a" }, { id: 1 });
  await settle();
  assert.deepStrictEqual(w.scans, [[1, "scan"]]);

  // The content script: tells the worker once, again when what it would
  // say has changed, and when asked.
  const sent = [];
  let found = [{ url: "https://a.example/feed", title: "A" }, { url: "https://a.example/feed", title: "again" }, { url: "javascript:void(0)", title: "" }];
  let asked, moved;
  const page = { href: "https://a.example/" };
  new Function("feedsOnPage", "location", "chrome", "window", "setTimeout", "setInterval", read("content.js"))(
    () => found, page,
    { runtime: { sendMessage: (m) => { sent.push(m); return Promise.resolve(); }, onMessage: { addListener: (fn) => { asked = fn; } } } },
    { navigation: { addEventListener: (_name, fn) => { moved = fn; } } },
    (fn) => fn(), () => {});
  assert.deepStrictEqual(sent, [{ type: "feeds", url: "https://a.example/", feeds: ["https://a.example/feed"] }]);
  page.href = "https://a.example/next";
  found = [];
  moved();
  assert.deepStrictEqual(sent[1], { type: "feeds", url: "https://a.example/next", feeds: [] });
  moved();
  assert.strictEqual(sent.length, 2);
  asked({ type: "scan" });
  assert.strictEqual(sent.length, 3);
  asked({ type: "something else" });
  assert.strictEqual(sent.length, 3);
  console.log("badge ok");
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
