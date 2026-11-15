import { api, type ItemRow } from "./api";
import { state, emitList, onList, fmtDate, toast } from "./state";
import { openItem } from "./reader";
import { markAllReadCurrent } from "./sidebar";

export async function loadItems(): Promise<void> {
  try {
    state.rows = await api.items({
      filter: state.filter,
      unread_only: state.unreadOnly,
      keep: [],
      limit: 200,
      offset: 0,
    });
  } catch (e) {
    toast(String(e), true);
    state.rows = [];
  }
  renderList();
}

export function renderList(): void {
  const root = document.getElementById("list")!;
  root.textContent = "";

  const header = document.createElement("div");
  header.id = "list-header";
  const title = document.createElement("span");
  title.textContent = state.filterLabel;
  const grow = document.createElement("div");
  grow.className = "grow";
  const unreadBtn = document.createElement("button");
  unreadBtn.className = "ghost";
  unreadBtn.textContent = state.unreadOnly ? "Unread ✓" : "Unread";
  unreadBtn.title = "Show unread only";
  unreadBtn.addEventListener("click", () => {
    state.unreadOnly = !state.unreadOnly;
    loadItems();
  });
  const markBtn = document.createElement("button");
  markBtn.className = "ghost";
  markBtn.textContent = "Mark all read";
  markBtn.addEventListener("click", () => void markAllReadCurrent().then(loadItems));
  header.append(title, grow, unreadBtn, markBtn);
  root.appendChild(header);

  if (state.rows.length === 0) {
    const empty = document.createElement("div");
    empty.className = "list-empty";
    empty.textContent = emptyText();
    root.appendChild(empty);
    return;
  }

  for (const row of state.rows) {
    root.appendChild(itemRowEl(row));
  }
}

function emptyText(): string {
  const f = state.filter.type;
  if (f === "saved") {
    return "Nothing saved yet.\nPaste a URL in the toolbar and press Save, or import an EPUB/PDF.";
  }
  if (state.status?.mode === "unconfigured") {
    return "No account configured.\nOpen Settings to add your FreshRSS account, or save a page from its URL.";
  }
  return "Nothing here.";
}

function itemRowEl(row: ItemRow): HTMLElement {
  const item = row.item;
  const el = document.createElement("div");
  el.className = "item-row";
  if (!item.read) el.classList.add("unread");
  if (item.id === state.currentItemId) el.classList.add("active");
  el.dataset.id = item.id;

  const star = document.createElement("button");
  star.className = "item-star" + (item.starred ? "" : " off");
  star.textContent = item.starred ? "★" : "☆";
  star.title = item.starred ? "Remove star" : "Star";
  star.addEventListener("click", (ev) => {
    ev.stopPropagation();
    void api.setStarred(item.id, !item.starred).then(loadItems).catch((e) => toast(String(e), true));
  });
  el.appendChild(star);

  const title = document.createElement("div");
  title.className = "item-title";
  title.textContent = item.title || "(untitled)";
  el.appendChild(title);

  const meta = document.createElement("div");
  meta.className = "item-meta";
  const parts = [row.source_title ?? "Saved"];
  if (item.kind === "chapter" && item.chapter_index !== null) parts.push(`ch. ${item.chapter_index}`);
  parts.push(fmtDate(item.published_at));
  meta.textContent = parts.join(" · ");
  el.appendChild(meta);

  if (item.excerpt) {
    const ex = document.createElement("div");
    ex.className = "item-excerpt";
    ex.textContent = item.excerpt;
    el.appendChild(ex);
  }

  el.addEventListener("click", () => void openItem(item.id));
  return el;
}

// Rows refresh their own read/star state from the reader via this.
export function patchItem(id: string, patch: { read?: boolean; starred?: boolean }): void {
  const row = state.rows.find((r) => r.item.id === id);
  if (!row) return;
  if (patch.read !== undefined) row.item.read = patch.read;
  if (patch.starred !== undefined) row.item.starred = patch.starred;
  emitList();
}

onList(renderList);
