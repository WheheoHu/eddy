import { api, type AiSettings, type ProviderSettings } from "./api";
import { state } from "./state";

const overlay = () => document.getElementById("settings-overlay")!;
const body = () => document.getElementById("settings-body")!;

export function openSettings(): void {
  overlay().hidden = false;
  void renderSettings();
}

export function closeSettings(): void {
  overlay().hidden = true;
}

function section(title: string, hint?: string): HTMLElement {
  const root = document.createElement("div");
  root.className = "settings-section";
  const h = document.createElement("h3");
  h.textContent = title;
  root.appendChild(h);
  if (hint) {
    const p = document.createElement("div");
    p.className = "hint";
    p.textContent = hint;
    root.appendChild(p);
  }
  body().appendChild(root);
  return root;
}

function fieldRow(label: string, type: string, value: string): { row: HTMLElement; input: HTMLInputElement } {
  const row = document.createElement("div");
  row.className = "form-row";
  const l = document.createElement("label");
  l.textContent = label;
  const input = document.createElement("input");
  input.type = type;
  input.value = value;
  if (type === "password") input.autocomplete = "off";
  row.append(l, input);
  return { row, input };
}

function note(parent: HTMLElement): { ok(msg: string): void; err(msg: string): void } {
  const el = document.createElement("div");
  parent.appendChild(el);
  return {
    ok(msg) { el.className = "ok-note"; el.textContent = msg; },
    err(msg) { el.className = "err-note"; el.textContent = msg; },
  };
}

// ---------- account ----------

function renderAccount(): void {
  const root = section("FreshRSS account", "Point Eddy at your FreshRSS (Google Reader compatible) server. The password goes into Windows Credential Manager.");
  const status = state.status;
  const n = note(root);
  if (status?.account) {
    const line = document.createElement("div");
    line.className = "account-line";
    line.textContent = `${status.account.username} @ ${status.account.base_url}`;
    root.appendChild(line);
    const remove = document.createElement("button");
    remove.textContent = "Remove account";
    remove.addEventListener("click", async () => {
      try {
        await api.removeAccount();
        n.ok("Account removed.");
        await renderSettings();
      } catch (e) { n.err(String(e)); }
    });
    root.appendChild(remove);
    return;
  }
  const url = fieldRow("Server", "text", "");
  const user = fieldRow("Username", "text", "");
  const pass = fieldRow("Password", "password", "");
  const connect = document.createElement("button");
  connect.className = "primary";
  connect.textContent = "Connect";
  connect.addEventListener("click", async () => {
    connect.disabled = true;
    try {
      await api.configureFreshrss(url.input.value.trim(), user.input.value.trim(), pass.input.value);
      n.ok("Connected.");
      await renderSettings();
    } catch (e) { n.err(String(e)); }
    connect.disabled = false;
  });
  root.append(url.row, user.row, pass.row, connect);
}

// ---------- server ----------

function renderServer(): void {
  const root = section("Sync server", "Optional: pair with an eddy-server instance to sync between devices. Get the pairing code from the server.");
  const n = note(root);
  const url = fieldRow("URL", "text", "");
  const code = fieldRow("Code", "text", "");
  const connect = document.createElement("button");
  connect.className = "primary";
  connect.textContent = "Pair";
  connect.addEventListener("click", async () => {
    connect.disabled = true;
    try {
      await api.connectServer(url.input.value.trim(), code.input.value.trim());
      n.ok("Paired.");
      await renderSettings();
    } catch (e) { n.err(String(e)); }
    connect.disabled = false;
  });
  const disconnect = document.createElement("button");
  disconnect.textContent = "Disconnect";
  disconnect.addEventListener("click", async () => {
    try {
      await api.disconnectServer();
      n.ok("Disconnected.");
      await renderSettings();
    } catch (e) { n.err(String(e)); }
  });
  root.append(url.row, code.row, connect, disconnect);
}

// ---------- storage ----------

async function renderStorage(): Promise<void> {
  const root = section("Storage", "Cached article images and files are evicted oldest-first beyond this limit.");
  const n = note(root);
  const usage = await api.cacheUsage().catch(() => 0);
  const line = document.createElement("div");
  line.className = "account-line";
  line.textContent = `Cache in use: ${(usage / 1024 / 1024 / 1024).toFixed(2)} GB`;
  root.appendChild(line);
  const gb = fieldRow("Limit (GB)", "number", "2");
  const save = document.createElement("button");
  save.textContent = "Apply";
  save.addEventListener("click", async () => {
    try {
      await api.setCacheLimit(Math.round(Number(gb.input.value || "2") * 1024 * 1024 * 1024));
      n.ok("Saved.");
    } catch (e) { n.err(String(e)); }
  });
  root.append(gb.row, save);
}

// ---------- AI ----------

interface ProviderBlock {
  fields: HTMLElement;
  keyInput: HTMLInputElement;
  get(): ProviderSettings;
}

function providerBlock(title: string, p: ProviderSettings | null): ProviderBlock {
  const fields = document.createElement("div");
  fields.style.marginBottom = "12px";
  const heading = document.createElement("div");
  heading.innerHTML = `<b>${title}</b>`;
  const proto = document.createElement("select");
  for (const [v, label] of [["auto", "Auto detect"], ["openai", "OpenAI compatible"], ["gemini", "Gemini"]] as const) {
    const opt = document.createElement("option");
    opt.value = v;
    opt.textContent = label;
    proto.appendChild(opt);
  }
  proto.value = p?.protocol ?? "auto";
  proto.style.marginBottom = "8px";
  const base = fieldRow("Base URL", "text", p?.base_url ?? "");
  const model = fieldRow("Model", "text", p?.model ?? "");
  const key = fieldRow("API key", "password", "");
  key.row.style.marginBottom = "0";
  fields.append(heading, proto, base.row, model.row, key.row);
  return {
    fields,
    keyInput: key.input,
    get: () => ({
      protocol: proto.value as ProviderSettings["protocol"],
      base_url: base.input.value.trim(),
      model: model.input.value.trim(),
      max_concurrency: p?.max_concurrency ?? 4,
    }),
  };
}

function renderAi(): void {
  const root = section("AI", "Client-side translation and summaries via an OpenAI-compatible or Gemini endpoint. Non-secret config syncs between devices; API keys stay in Credential Manager on this device.");
  const n = note(root);
  const wrap = document.createElement("div");
  root.appendChild(wrap);

  void (async () => {
    let settings: AiSettings;
    try {
      settings = await api.aiSettings();
    } catch (e) {
      n.err(String(e));
      return;
    }
    wrap.textContent = "";
    const lang = fieldRow("Target", "text", settings.target_language);
    const tBlock = providerBlock("Translation", settings.translation_provider);
    const sBlock = providerBlock("Summaries & tags", settings.summary_provider);
    const save = document.createElement("button");
    save.className = "primary";
    save.style.marginTop = "10px";
    save.textContent = "Save AI settings";
    save.addEventListener("click", async () => {
      const next: AiSettings = { ...settings };
      next.target_language = lang.input.value.trim() || settings.target_language;
      const t = tBlock.get();
      const s = sBlock.get();
      next.translation_provider = t.base_url ? t : null;
      next.summary_provider = s.base_url ? s : null;
      try {
        await api.setAiSettings(next);
        const tKey = tBlock.keyInput.value;
        const sKey = sBlock.keyInput.value;
        if (tKey) await api.setAiKey("translation", tKey);
        if (sKey) await api.setAiKey("summary", sKey);
        n.ok("Saved. Keys stay on this device.");
      } catch (e) { n.err(String(e)); }
    });
    wrap.append(lang.row, tBlock.fields, sBlock.fields, save);
  })();
}

async function renderSettings(): Promise<void> {
  body().textContent = "";
  renderAccount();
  renderServer();
  await renderStorage();
  renderAi();
}

export function initSettings(): void {
  document.getElementById("settings-btn")!.addEventListener("click", openSettings);
  document.getElementById("settings-close")!.addEventListener("click", closeSettings);
  overlay().addEventListener("click", (ev) => {
    if (ev.target === overlay()) closeSettings();
  });
  document.getElementById("summary-close")!.addEventListener("click", () => {
    document.getElementById("summary-panel")!.hidden = true;
  });
}
