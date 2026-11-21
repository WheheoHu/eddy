const rsshub = document.getElementById("rsshub");
const key = document.getElementById("key");
const status = document.getElementById("status");
const save = document.getElementById("save");
const FALLBACK = "https://rsshub.app";

function describe(stored) {
  if (stored.rules && stored.rulesAt) {
    const when = new Date(stored.rulesAt).toLocaleString();
    status.textContent = `${countRules(stored.rules)} 条规则，更新于 ${when}` + (stored.rulesError ? `（上次更新失败：${stored.rulesError}）` : "");
  } else {
    status.textContent = stored.rulesError ? `规则未加载：${stored.rulesError}` : "规则未加载";
  }
}

chrome.storage.local.get({ rsshub: FALLBACK, key: "", rules: null, rulesAt: 0, rulesError: "" }).then((stored) => {
  rsshub.value = stored.rsshub;
  key.value = stored.key;
  describe(stored);
});

save.onclick = async () => {
  let base;
  try {
    base = new URL(rsshub.value.trim() || FALLBACK);
    if (!/^https?:$/.test(base.protocol)) throw new Error();
  } catch (e) {
    status.textContent = "请输入完整的地址，例如 https://rsshub.example.com";
    return;
  }
  save.disabled = true;
  status.textContent = "正在更新规则…";
  // Reading the rules from an instance of your own needs the browser's
  // leave to reach it; rsshub.app is allowed from the start.
  const origin = `${base.protocol}//${base.host}/*`;
  let allowed = await chrome.permissions.contains({ origins: [origin] });
  if (!allowed) {
    try {
      allowed = await chrome.permissions.request({ origins: [origin] });
    } catch (e) {
      allowed = false;
    }
  }
  if (!allowed) {
    status.textContent = "没有访问这个地址的权限，规则无法更新";
    save.disabled = false;
    return;
  }
  await chrome.storage.local.set({ rsshub: base.href.replace(/\/+$/, ""), key: key.value.trim() });
  chrome.runtime.sendMessage({ type: "refreshRules" }, async (reply) => {
    save.disabled = false;
    if (reply?.ok) {
      describe(await chrome.storage.local.get({ rules: null, rulesAt: 0, rulesError: "" }));
    } else {
      status.textContent = `规则更新失败：${reply?.error || "未知错误"}`;
    }
  });
};
