const rsshub = document.getElementById("rsshub");
const key = document.getElementById("key");
const status = document.getElementById("status");
const rulesLine = document.getElementById("rules");
const buttons = ["test", "save", "update"].map((id) => document.getElementById(id));
const FALLBACK = "https://rsshub.app";

document.getElementById("version").textContent = `Version ${chrome.runtime.getManifest().version}`;

function say(text, tone = "") {
  status.textContent = text;
  status.className = tone;
}

function busy(on) {
  for (const button of buttons) button.disabled = on;
}

// What is kept: how many rules, from where, since when, and whether the
// last try to refresh them failed.
async function describeRules() {
  const stored = await chrome.storage.local.get({ rules: null, rulesAt: 0, rulesFrom: "", rulesError: "" });
  if (stored.rules && stored.rulesAt) {
    const from = stored.rulesFrom.replace(/^https?:\/\//, "");
    rulesLine.textContent =
      `${countRules(stored.rules).toLocaleString()} rules from ${from}, updated ${new Date(stored.rulesAt).toLocaleString()}` +
      (stored.rulesError ? `. The last update failed: ${stored.rulesError}` : "");
  } else {
    rulesLine.textContent = stored.rulesError ? `No rules yet: ${stored.rulesError}` : "No rules yet";
  }
}

// The address in the field as an instance's root, or null with a word said.
function typedInstance() {
  try {
    const url = new URL(rsshub.value.trim() || FALLBACK);
    if (!/^https?:$/.test(url.protocol)) throw new Error();
    return url.href.replace(/\/+$/, "");
  } catch (e) {
    say("Enter a full address, such as https://rsshub.example.com", "bad");
    return null;
  }
}

// The browser's leave to read from an instance other than rsshub.app,
// asked for the first time it is used. Needs the click it is called from.
async function allowed(base) {
  const origins = [new URL(base).origin + "/*"];
  if (await chrome.permissions.contains({ origins })) return true;
  try {
    return await chrome.permissions.request({ origins });
  } catch (e) {
    return false;
  }
}

// Asks the instance in the fields for its rules and says what came back.
// Nothing is saved.
async function test() {
  const base = typedInstance();
  if (!base) return;
  busy(true);
  say("Testing…");
  if (!(await allowed(base))) {
    say("Chrome was not allowed to read from this address.", "bad");
  } else {
    const started = performance.now();
    try {
      const rules = await fetchRules(base, key.value.trim());
      const count = countRules(rules);
      const took = Math.round(performance.now() - started);
      if (count > 0) say(`It works: ${count.toLocaleString()} rules, answered in ${took} ms.`, "good");
      else say("The instance answered, with no rules.", "bad");
    } catch (e) {
      say(`${e.message}.`, "bad");
    }
  }
  busy(false);
}

// Keeps the instance and key, and fetches its rules.
async function save() {
  const base = typedInstance();
  if (!base) return;
  busy(true);
  say("Saving…");
  if (!(await allowed(base))) {
    say("Chrome was not allowed to read from this address, so nothing was saved.", "bad");
    busy(false);
    return;
  }
  await chrome.storage.local.set({ rsshub: base, key: key.value.trim() });
  rsshub.value = base;
  await update("Saved.");
}

function update(done = "Rules updated.") {
  busy(true);
  if (!status.textContent || status.className) say("Updating rules…");
  return new Promise((finished) => {
    chrome.runtime.sendMessage({ type: "refreshRules" }, async (reply) => {
      if (reply?.ok) say(`${done} ${reply.count.toLocaleString()} rules.`, "good");
      else say(`${done === "Saved." ? "Saved, but the rules" : "The rules"} couldn’t be updated: ${reply?.error || "unknown error"}.`, "bad");
      await describeRules();
      busy(false);
      finished();
    });
  });
}

document.getElementById("test").onclick = test;
document.getElementById("save").onclick = save;
document.getElementById("update").onclick = () => update();
for (const field of [rsshub, key]) {
  field.addEventListener("keydown", (event) => {
    if (event.key === "Enter") save();
  });
  field.addEventListener("input", () => say(""));
}

// The number on the toolbar icon: kept as soon as it is switched, and the
// worker hears of it from the storage itself.
const badge = document.getElementById("badge");
badge.onchange = () => chrome.storage.local.set({ badge: badge.checked });

chrome.storage.local.get({ rsshub: FALLBACK, key: "", badge: true }).then((stored) => {
  rsshub.value = stored.rsshub;
  key.value = stored.key;
  badge.checked = stored.badge;
});
describeRules();
