import { api } from "./api";
import { toast } from "./state";
import { openItem } from "./reader";

const overlay = () => document.getElementById("search-overlay")!;
const input = () => document.getElementById("search-input") as HTMLInputElement;
const results = () => document.getElementById("search-results")!;

export function openSearch(): void {
  overlay().hidden = false;
  input().value = "";
  results().textContent = "";
  input().focus();
}

export function closeSearch(): void {
  overlay().hidden = true;
}

function snippetHtml(snippet: string, highlights: [number, number][]): DocumentFragment {
  const frag = document.createDocumentFragment();
  let pos = 0;
  for (const [start, end] of highlights) {
    if (start > pos) frag.append(snippet.slice(pos, start));
    if (end > start) {
      const b = document.createElement("b");
      b.textContent = snippet.slice(start, end);
      frag.append(b);
      pos = end;
    }
  }
  if (pos < snippet.length) frag.append(snippet.slice(pos));
  return frag;
}

async function runSearch(text: string): Promise<void> {
  results().textContent = "";
  const hint = document.createElement("div");
  hint.className = "list-empty";
  hint.textContent = "Searching…";
  results().appendChild(hint);
  try {
    const rows = await api.searchAll(text, 50);
    results().textContent = "";
    if (rows.length === 0) {
      const empty = document.createElement("div");
      empty.className = "list-empty";
      empty.textContent = "No matches.";
      results().appendChild(empty);
      return;
    }
    for (const row of rows) {
      const hit = document.createElement("div");
      hit.className = "search-hit";
      const t = document.createElement("div");
      t.className = "t";
      t.textContent = row.item.item.title || "(untitled)";
      const m = document.createElement("div");
      m.className = "m";
      m.textContent = row.item.source_title ?? "Saved";
      const s = document.createElement("div");
      s.className = "s";
      s.append(snippetHtml(row.snippet, row.highlights));
      hit.append(t, m, s);
      hit.addEventListener("click", () => {
        closeSearch();
        void openItem(row.item.item.id);
      });
      results().appendChild(hit);
    }
  } catch (e) {
    results().textContent = "";
    toast(String(e), true);
  }
}

export function initSearch(): void {
  document.getElementById("search-btn")!.addEventListener("click", openSearch);
  overlay().addEventListener("click", (ev) => {
    if (ev.target === overlay()) closeSearch();
  });
  let debounce = 0;
  input().addEventListener("input", () => {
    window.clearTimeout(debounce);
    const text = input().value.trim();
    if (text.length < 2) return;
    debounce = window.setTimeout(() => void runSearch(text), 300);
  });
  input().addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") void runSearch(input().value.trim());
    if (ev.key === "Escape") closeSearch();
  });
}
