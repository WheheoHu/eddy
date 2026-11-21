importScripts("radar.js", "page-feeds.js");

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

// The number on the toolbar icon: how many feeds the tab's page has, its
// own (as its content script counted them) and RSSHub's together. No
// number for none, or when the setting is off.
async function showCount(tabId, url, pageFeeds) {
  if (tabId === undefined) return 0;
  const { badge } = await chrome.storage.local.get({ badge: true });
  let count = 0;
  if (badge) {
    const made = await radar(url);
    count = new Set([...pageFeeds, ...made.feeds.map((feed) => feed.url)]).size;
  }
  try {
    await chrome.action.setBadgeBackgroundColor({ tabId, color: "#0a84ff" });
    await chrome.action.setBadgeTextColor?.({ tabId, color: "#ffffff" });
    await chrome.action.setBadgeText({ tabId, text: count ? String(count) : "" });
    await chrome.action.setTitle({ tabId, title: count ? `Save to Eddy · ${count} feed${count === 1 ? "" : "s"} on this page` : "Save to Eddy" });
  } catch (e) {
    // The tab closed meanwhile.
  }
  return count;
}

// Has a tab's page count its feeds again; without a content script there
// (a page from before the extension, one it can't run on) the number goes.
function recount(tabId) {
  if (tabId === undefined) return;
  chrome.tabs.sendMessage(tabId, { type: "scan" }).catch(() => chrome.action.setBadgeText({ tabId, text: "" }).catch(() => {}));
}

// The setting switched: every tab's number goes, or is counted afresh.
chrome.storage.onChanged.addListener((changes, area) => {
  if (area !== "local" || !("badge" in changes)) return;
  chrome.tabs.query({}).then((tabs) => tabs.forEach((tab) => recount(tab.id)));
});

// A tick or a mark for a moment after saving from the right-click menu,
// then back to the page's number.
async function flash(tabId, ok, error) {
  await chrome.action.setBadgeBackgroundColor({ tabId, color: ok ? "#2f9e44" : "#c92a2a" });
  await chrome.action.setBadgeText({ tabId, text: ok ? "✓" : "!" });
  await chrome.action.setTitle({ tabId, title: ok ? "Saved to Eddy" : `Couldn’t save: ${error || "unknown error"}` });
  setTimeout(() => recount(tabId), 2500);
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
chrome.runtime.onMessage.addListener((message, sender, respond) => {
  // A page's content script, saying which feeds the page has.
  if (message.type === "feeds") {
    showCount(sender.tab?.id, message.url, Array.isArray(message.feeds) ? message.feeds : []);
    return false;
  }
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
