importScripts("radar.js");

const HOST = "dev.eddy.capture";

// RSSHub: the instance whose feeds are offered, and the rules it serves for
// finding them. rsshub.app is the public one, meant for trying things out;
// the settings page takes another.
const DEFAULTS = { rsshub: "https://rsshub.app", key: "", rules: null, rulesAt: 0, rulesFrom: "", rulesError: "" };
const RULES_MAX_AGE = 24 * 60 * 60 * 1000;

function settings() {
  return chrome.storage.local.get(DEFAULTS);
}

// Fetches the instance's rules and keeps them. A failure leaves the rules
// already here in place and is remembered for the settings page to show.
let refreshing = null;
function refreshRules() {
  refreshing ??= (async () => {
    const { rsshub, key } = await settings();
    const base = rsshub.replace(/\/+$/, "");
    try {
      const rules = await fetchRules(base, key);
      await chrome.storage.local.set({ rules, rulesAt: Date.now(), rulesFrom: base, rulesError: "" });
      return { ok: true, count: countRules(rules) };
    } catch (e) {
      const error = String(e?.message || e);
      await chrome.storage.local.set({ rulesError: error });
      return { ok: false, error };
    }
  })().finally(() => {
    refreshing = null;
  });
  return refreshing;
}

// The feeds the instance can make for a page. Rules that are missing, a
// day old or another instance's are fetched again: waited for (briefly)
// when there are none to go on, in the background otherwise.
async function radar(url) {
  let { rsshub, key, rules, rulesAt, rulesFrom } = await settings();
  const base = rsshub.replace(/\/+$/, "");
  const stale = rulesFrom !== base || Date.now() - rulesAt > RULES_MAX_AGE;
  if (!rules || rulesFrom !== base) {
    await Promise.race([refreshRules(), new Promise((done) => setTimeout(done, 6000))]);
    ({ rules, rulesFrom } = await settings());
    if (rulesFrom !== base) rules = null;
  } else if (stale) {
    refreshRules();
  }
  return { feeds: radarFeeds(rules, url, base, key), loaded: !!rules };
}

function send(message) {
  return new Promise((resolve) => {
    chrome.runtime.sendNativeMessage(HOST, message, (reply) => {
      if (chrome.runtime.lastError) {
        resolve({ ok: false, error: chrome.runtime.lastError.message });
      } else {
        resolve(reply || { ok: false, error: "no reply" });
      }
    });
  });
}

async function flash(tabId, ok, error) {
  await chrome.action.setBadgeBackgroundColor({ tabId, color: ok ? "#2f9e44" : "#c92a2a" });
  await chrome.action.setBadgeText({ tabId, text: ok ? "✓" : "!" });
  await chrome.action.setTitle({ tabId, title: ok ? "Saved to Eddy" : `Couldn’t save: ${error || "unknown error"}` });
  setTimeout(() => chrome.action.setBadgeText({ tabId, text: "" }), 2500);
}

function isPdf(tab) {
  return /\.pdf($|[?#])/i.test(tab.url || "");
}

// Hands the tab to Eddy; the reply carries the address saved, for "Show".
async function capture(tab) {
  if (!tab || !/^https?:/.test(tab.url || "")) {
    return { ok: false, error: "Only web pages can be saved." };
  }
  // Chrome's PDF viewer has no page DOM; Eddy downloads the file itself.
  if (isPdf(tab)) {
    const reply = await send({ url: tab.url, title: tab.title });
    return { ...reply, url: tab.url };
  }
  let page;
  try {
    const [result] = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => ({
        url: location.href,
        title: document.title,
        html: "<!doctype html>\n" + document.documentElement.outerHTML,
      }),
    });
    page = result.result;
  } catch (e) {
    page = { url: tab.url, title: tab.title };
  }
  const reply = await send(page);
  return { ...reply, url: page.url };
}

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

// What the popup offers before anything is saved: the page it was opened
// on, the feeds the page names, and the ones RSSHub can make for it.
async function inspect(tab) {
  if (!tab || !/^https?:/.test(tab.url || "")) {
    return { ok: false, error: "Only web pages can be saved." };
  }
  const page = { ok: true, url: tab.url, title: tab.title || tab.url, feeds: [], radar: [], radarLoaded: false };
  const found = radar(tab.url);
  if (!isPdf(tab)) {
    try {
      const [result] = await chrome.scripting.executeScript({ target: { tabId: tab.id }, func: feedsOnPage });
      const seen = new Set();
      for (const feed of result.result || []) {
        if (!/^https?:/.test(feed.url) || seen.has(feed.url)) continue;
        seen.add(feed.url);
        page.feeds.push(feed);
      }
      page.feeds = page.feeds.slice(0, 8);
    } catch (e) {
      // A page scripts can't run on (the Web Store, a viewer): saving its
      // address still works.
    }
  }
  const made = await found;
  const known = new Set(page.feeds.map((feed) => feed.url));
  page.radar = made.feeds.filter((feed) => !known.has(feed.url));
  page.radarLoaded = made.loaded;
  return page;
}

// The popup asks what is here, then saves the tab it was opened on or hands
// one of its feeds to Eddy, as its buttons are pressed; and asks for "Show".
chrome.runtime.onMessage.addListener((message, _sender, respond) => {
  if (message.type === "inspect") {
    chrome.tabs.query({ active: true, currentWindow: true }).then(([tab]) => inspect(tab)).then(respond);
    return true;
  }
  if (message.type === "refreshRules") {
    refreshRules().then(respond);
    return true;
  }
  if (message.type === "subscribe") {
    send({ feed: message.url, title: message.title }).then(respond);
    return true;
  }
  if (message.type === "capture") {
    chrome.tabs.query({ active: true, currentWindow: true }).then(([tab]) => capture(tab)).then(respond);
    return true;
  }
  if (message.type === "show") {
    send({ show: message.url }).then(respond);
    return true;
  }
  return false;
});

chrome.runtime.onInstalled.addListener(() => {
  refreshRules();
  chrome.contextMenus.create({ id: "eddy-save-link", title: "Read This Link Later in Eddy", contexts: ["link"] });
  chrome.contextMenus.create({ id: "eddy-save-page", title: "Save This Page to Eddy", contexts: ["page"] });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId === "eddy-save-link" && info.linkUrl) {
    const reply = await send({ url: info.linkUrl });
    await flash(tab?.id, reply.ok, reply.error);
  } else if (info.menuItemId === "eddy-save-page") {
    const reply = await capture(tab);
    await flash(tab?.id, reply.ok, reply.error);
  }
});
