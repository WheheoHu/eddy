// node extensions/tests/options.test.js
// Runs the settings page's script, and the rules fetch it shares with the
// worker, against stand-ins for the page, the browser and the network.
const fs = require("fs");
const path = require("path");
const assert = require("assert");
const radar = require("../chrome/radar.js");
const src = fs.readFileSync(path.join(__dirname, "../chrome/options.js"), "utf8");

const RULES = { "example.com": { _name: "Example", ".": [{ title: "A", source: ["/a"], target: "/example/a" }, { title: "B", source: ["/b"], target: "/example/b" }] } };

function page({ answer = { ok: true, status: 200, json: async () => RULES }, permitted = true, granted = true, refresh = { ok: true, count: 2 } } = {}) {
  const els = {}, stored = { rsshub: "https://rsshub.app", key: "" }, asked = [], fetched = [];
  const el = (id) => (els[id] ??= { id, value: "", textContent: "", className: "", disabled: false, addEventListener() {} });
  const chrome = {
    runtime: {
      getManifest: () => ({ version: "9.9.9" }),
      sendMessage(message, reply) { asked.push(message.type); reply(refresh); },
    },
    storage: { local: {
      get: async (defaults) => ({ ...defaults, ...stored }),
      set: async (values) => { Object.assign(stored, values); },
    } },
    permissions: {
      contains: async () => permitted,
      request: async ({ origins }) => { asked.push("permission " + origins[0]); return granted; },
    },
  };
  const fetch = async (url) => {
    fetched.push(url);
    if (answer instanceof Error) throw answer;
    return answer;
  };
  // The shared fetch runs with this stand-in network.
  const fetchRules = new Function("fetch", "AbortSignal", radar.fetchRules.toString() + "; return fetchRules;")(fetch, { timeout: () => undefined });
  new Function("document", "chrome", "performance", "countRules", "fetchRules", src)({ getElementById: el }, chrome, { now: () => 0 }, radar.countRules, fetchRules);
  return { els, stored, asked, fetched };
}
const settle = () => new Promise((done) => setTimeout(done, 5));

(async () => {
  // Opening it shows what is kept.
  let p = page();
  await settle();
  assert.strictEqual(p.els.rsshub.value, "https://rsshub.app");
  assert.strictEqual(p.els.version.textContent, "Version 9.9.9");
  assert.strictEqual(p.els.rules.textContent, "No rules yet");

  // Test asks the instance typed in, key included, and saves nothing.
  p.els.rsshub.value = " https://hub.example/ ";
  p.els.key.value = "s3cret";
  await p.els.test.onclick();
  assert.deepStrictEqual(p.fetched, ["https://hub.example/api/radar/rules?key=s3cret"]);
  assert.strictEqual(p.els.status.textContent, "It works: 2 rules, answered in 0 ms.");
  assert.strictEqual(p.els.status.className, "good");
  assert.strictEqual(p.stored.rsshub, "https://rsshub.app");
  assert.strictEqual(p.els.test.disabled, false);

  // What can go wrong is said in a few words.
  for (const [answer, words] of [
    [{ ok: false, status: 503 }, "HTTP 503."],
    [new TypeError("Failed to fetch"), "Couldn’t reach the instance."],
    [{ ok: true, status: 200, json: async () => { throw new SyntaxError("x"); } }, "The answer is not a list of rules."],
    [{ ok: true, status: 200, json: async () => [] }, "The answer is not a list of rules."],
    [{ ok: true, status: 200, json: async () => ({}) }, "The instance answered, with no rules."],
  ]) {
    p = page({ answer });
    await settle();
    await p.els.test.onclick();
    assert.strictEqual(p.els.status.textContent, words);
    assert.strictEqual(p.els.status.className, "bad");
  }
  p = page();
  await settle();
  p.els.rsshub.value = "not an address";
  await p.els.test.onclick();
  assert.match(p.els.status.textContent, /^Enter a full address/);
  assert.deepStrictEqual(p.fetched, []);

  // An instance of one's own needs Chrome's leave, asked for on the click.
  p = page({ permitted: false, granted: false });
  await settle();
  p.els.rsshub.value = "https://hub.example";
  await p.els.test.onclick();
  assert.deepStrictEqual(p.asked, ["permission https://hub.example/*"]);
  assert.match(p.els.status.textContent, /not allowed/);
  await p.els.save.onclick();
  assert.strictEqual(p.stored.rsshub, "https://rsshub.app", "nothing saved without it");

  // Save keeps the instance and has the worker fetch its rules.
  p = page({ permitted: false, granted: true });
  await settle();
  p.els.rsshub.value = "https://hub.example/";
  p.els.key.value = " k ";
  await p.els.save.onclick();
  assert.deepStrictEqual({ rsshub: p.stored.rsshub, key: p.stored.key }, { rsshub: "https://hub.example", key: "k" });
  assert.deepStrictEqual(p.asked, ["permission https://hub.example/*", "refreshRules"]);
  assert.strictEqual(p.els.status.textContent, "Saved. 2 rules.");
  assert.strictEqual(p.els.rsshub.value, "https://hub.example");

  // Saved, with rules that would not come: both are said.
  p = page({ refresh: { ok: false, error: "HTTP 500" } });
  await settle();
  await p.els.save.onclick();
  assert.strictEqual(p.els.status.textContent, "Saved, but the rules couldn’t be updated: HTTP 500.");
  await p.els.update.onclick();
  assert.strictEqual(p.els.status.textContent, "The rules couldn’t be updated: HTTP 500.");
  console.log("options ok");
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
