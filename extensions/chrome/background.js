const HOST = "dev.eddy.capture";

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
  await chrome.action.setTitle({ tabId, title: ok ? "已保存到 Eddy" : `保存失败：${error || "未知错误"}` });
  setTimeout(() => chrome.action.setBadgeText({ tabId, text: "" }), 2500);
}

function isPdf(tab) {
  return /\.pdf($|[?#])/i.test(tab.url || "");
}

// Hands the tab to Eddy; the reply carries the address saved, for "Show".
async function capture(tab) {
  if (!tab || !/^https?:/.test(tab.url || "")) {
    return { ok: false, error: "只能保存网页" };
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

// The popup saves the tab it was opened on, and asks for "Show".
chrome.runtime.onMessage.addListener((message, _sender, respond) => {
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
  chrome.contextMenus.create({ id: "eddy-save-link", title: "用 Eddy 稍后阅读这个链接", contexts: ["link"] });
  chrome.contextMenus.create({ id: "eddy-save-page", title: "保存这个页面到 Eddy", contexts: ["page"] });
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
