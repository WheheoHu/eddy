import { api, onEvent, onTranslation, type TranslationBatch } from "./api";
import { state, reloadSnapshot, toast } from "./state";
import { loadItems } from "./list";
import { initSearch } from "./search";
import { initSettings } from "./settings";
import { applyTranslation } from "./reader";

// ---------- top bar actions ----------

async function savePage(): Promise<void> {
  const input = document.getElementById("save-input") as HTMLInputElement;
  const url = input.value.trim();
  if (!url) { toast("Paste a URL first"); return; }
  try {
    const row = await api.saveUrl(url);
    input.value = "";
    toast(`Saved: ${row.item.title}`);
    await loadItems();
    await reloadSnapshot();
  } catch (e) {
    toast(String(e), true);
  }
}

async function refresh(): Promise<void> {
  const btn = document.getElementById("refresh-btn") as HTMLButtonElement;
  btn.disabled = true;
  try {
    const summary = await api.refresh();
    if (summary.new_items > 0) toast(`New articles: ${summary.new_items}`);
    else toast("Nothing new");
    await loadItems();
    await reloadSnapshot();
  } catch (e) {
    toast(String(e), true);
  }
  btn.disabled = false;
}

async function importFile(): Promise<void> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const path = await open({
    multiple: false,
    filters: [{ name: "Documents", extensions: ["epub", "pdf"] }],
  });
  if (!path) return;
  try {
    const imported = await api.importFile(path);
    toast(imported.existing ? "Already imported" : `Imported: ${imported.title}`);
    await loadItems();
    await reloadSnapshot();
  } catch (e) {
    toast(String(e), true);
  }
}

// ---------- sync badge ----------

async function updateSyncBadge(): Promise<void> {
  const badge = document.getElementById("sync-badge")!;
  try {
    const status = await api.status();
    state.status = status;
    if (status.sync) {
      const pct = status.sync.total ? Math.round((status.sync.done / status.sync.total) * 100) : status.sync.done;
      badge.textContent = `${status.sync.phase.replace("_", " ")} ${pct}%`;
      badge.hidden = false;
    } else {
      badge.hidden = true;
    }
  } catch { /* status polling is best-effort */ }
}

// ---------- bootstrap ----------

async function main(): Promise<void> {
  initSearch();
  initSettings();

  document.getElementById("save-btn")!.addEventListener("click", () => void savePage());
  document.getElementById("save-input")!.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") void savePage();
  });
  document.getElementById("refresh-btn")!.addEventListener("click", () => void refresh());
  document.getElementById("import-btn")!.addEventListener("click", () => void importFile());

  // Engine pulses: "change" reloads the snapshot (debounced, like AppModel),
  // "progress" only updates the sync badge.
  let pending = 0;
  void onEvent("eddy:change", () => {
    window.clearTimeout(pending);
    pending = window.setTimeout(() => {
      void reloadSnapshot().then(loadItems);
    }, 350);
  });
  void onEvent("eddy:progress", () => void updateSyncBadge());
  void onTranslation((batch: TranslationBatch) => {
    if (batch.item_id === state.currentItemId) applyTranslation(batch.segments);
  });

  await reloadSnapshot();
  await loadItems();

  // Periodic quiet refresh, mirroring the macOS app's 15-minute fallback.
  window.setInterval(() => { void refresh(); }, 15 * 60 * 1000);

  const pdfOk = await api.pdf_available().catch(() => false);
  const importBtn = document.getElementById("import-btn") as HTMLButtonElement;
  if (!pdfOk) {
    importBtn.title = "pdfium.dll missing next to the exe — PDF import disabled";
    importBtn.style.opacity = "0.5";
  }
}

void main();
