const ask = document.getElementById("ask");
const result = document.getElementById("result");
const saveButton = document.getElementById("save");
const feeds = document.getElementById("feeds");
const status = document.getElementById("status");
const detail = document.getElementById("detail");
const showButton = document.getElementById("show");
const copyButton = document.getElementById("copy");

async function copyError() {
  await navigator.clipboard.writeText(detail.textContent);
  copyButton.textContent = "已复制";
}

// From asking to telling: the buttons give way to what happened.
function tell(text) {
  ask.hidden = true;
  result.hidden = false;
  status.textContent = text;
}

function failed(what, error) {
  tell(what);
  detail.textContent = error || "未知错误";
  copyButton.hidden = false;
  copyButton.onclick = copyError;
  detail.onclick = copyError;
}

function save() {
  tell("正在保存到 Eddy…");
  chrome.runtime.sendMessage({ type: "capture" }, (reply) => {
    if (!reply || !reply.ok) return failed("保存失败", reply?.error);
    status.textContent = "已保存到 Eddy";
    showButton.hidden = false;
    showButton.focus();
    showButton.onclick = () => {
      showButton.disabled = true;
      chrome.runtime.sendMessage({ type: "show", url: reply.url }, (shown) => {
        if (shown?.ok) {
          window.close();
        } else {
          showButton.hidden = true;
          failed("保存失败", shown?.error);
        }
      });
    };
  });
}

// Eddy asks which folder and confirms; the popup has done its part once
// Eddy is up.
function subscribe(feed, button) {
  button.disabled = true;
  chrome.runtime.sendMessage({ type: "subscribe", url: feed.url, title: feed.title }, (reply) => {
    if (reply?.ok) {
      window.close();
    } else {
      failed("没能打开 Eddy", reply?.error);
    }
  });
}

function feedRow(text, hint, action, feed) {
  const row = document.createElement("div");
  row.className = "row feed";
  const name = document.createElement("span");
  name.textContent = text;
  name.title = hint;
  const button = document.createElement("button");
  button.textContent = action;
  button.onclick = () => subscribe(feed, button);
  row.append(name, button);
  return row;
}

function feedName(feed, page) {
  if (feed.title) return feed.title;
  try {
    const url = new URL(feed.url);
    return url.host === new URL(page.url).host ? url.pathname : url.host + url.pathname;
  } catch (e) {
    return feed.url;
  }
}

chrome.runtime.sendMessage({ type: "inspect" }, (page) => {
  if (!page || !page.ok) return failed("不能保存这个页面", page?.error);
  document.getElementById("title").textContent = page.title;
  try {
    document.getElementById("host").textContent = new URL(page.url).host;
  } catch (e) {}
  for (const feed of page.feeds) {
    feeds.append(feedRow(feedName(feed, page), feed.url, "订阅…", feed));
  }
  // Many sites have a feed and don't say so in the page (少数派 is one).
  // Eddy's sheet opens on the page's own address, to try or to correct.
  if (page.feeds.length === 0 && !(page.radar || []).length) {
    feeds.append(feedRow("这个页面没有标明订阅源", "", "手动添加…", { url: page.url, title: "" }));
  }
  feeds.hidden = page.feeds.length === 0 && (page.radar || []).length > 0;
  // Feeds the page does not have of its own, that RSSHub makes for it.
  const radar = document.getElementById("radar");
  for (const feed of page.radar || []) {
    radar.append(feedRow(feed.title || feed.url, feed.url, "订阅…", feed));
  }
  radar.hidden = !(page.radar || []).length;
  if (!page.radarLoaded) document.getElementById("radar-note").textContent = "RSSHub 规则未加载";
  document.getElementById("options").onclick = (event) => {
    event.preventDefault();
    chrome.runtime.openOptionsPage();
  };
  ask.hidden = false;
  saveButton.onclick = save;
  // ⌘⇧E, then Return, saves.
  saveButton.focus();
});
