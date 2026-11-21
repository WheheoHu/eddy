// Tells the worker which feeds this page has, for the number on the toolbar
// icon. Only addresses of feeds leave the page, and only to the extension's
// own worker.
(() => {
  let told = "";

  function tell() {
    let feeds = [];
    try {
      feeds = feedsOnPage();
    } catch (e) {}
    const urls = [...new Set(feeds.map((feed) => feed.url).filter((url) => /^https?:/.test(url)))].slice(0, 8);
    const now = [location.href, ...urls].join("\n");
    if (now === told) return;
    told = now;
    try {
      chrome.runtime.sendMessage({ type: "feeds", url: location.href, feeds: urls }).catch(() => {});
    } catch (e) {
      // The extension was reloaded under this page; the next page has a
      // script that can talk to it.
    }
  }

  tell();
  // Pages that fill in their head after loading.
  setTimeout(tell, 2500);
  // Pages that go somewhere else without loading (the Navigation API
  // says when; where it is missing, the address is looked at now and then).
  const moved = () => setTimeout(tell, 800);
  if (window.navigation?.addEventListener) {
    window.navigation.addEventListener("navigatesuccess", moved);
  } else {
    let last = location.href;
    setInterval(() => {
      if (location.href !== last) {
        last = location.href;
        moved();
      }
    }, 1500);
  }
  // The worker asks again after it has shown something else on the icon,
  // or when the setting is switched on.
  try {
    chrome.runtime.onMessage.addListener((message) => {
      if (message?.type === "scan") {
        told = "";
        tell();
      }
    });
  } catch (e) {}
})();
