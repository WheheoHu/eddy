import { api, type Filter } from "./api";
import { state, emitList, onSidebar, toast } from "./state";

export function setFilter(filter: Filter, label: string): void {
  state.filter = filter;
  state.filterLabel = label;
  state.currentItemId = null;
  state.doc = null;
  emitList();
  renderSidebar();
}

export function renderSidebar(): void {
  const root = document.getElementById("sidebar")!;
  root.textContent = "";
  const is = (f: Filter, label: string, name: string) =>
    state.filter.type === f.type && (f.type === "source" ? state.filterLabel === label : true) && name;

  const addGroup = (title: string) => {
    const g = document.createElement("div");
    g.className = "sidebar-group";
    const t = document.createElement("div");
    t.className = "sidebar-title";
    t.textContent = title;
    g.appendChild(t);
    root.appendChild(g);
    return g;
  };

  const addRow = (group: HTMLElement, label: string, filter: Filter, badge?: number, cls?: string) => {
    const row = document.createElement("div");
    row.className = "sidebar-row";
    if (is(filter, label, "x")) row.classList.add("active");
    if (cls) row.classList.add(cls);
    const l = document.createElement("span");
    l.className = "label";
    l.textContent = label;
    row.appendChild(l);
    if (badge !== undefined && badge > 0) {
      const b = document.createElement("span");
      b.className = "badge";
      b.textContent = String(badge > 999 ? "999+" : badge);
      row.appendChild(b);
    }
    row.addEventListener("click", () => setFilter(filter, label));
    group.appendChild(row);
  };

  const views = addGroup("Library");
  addRow(views, "All articles", { type: "all" });
  addRow(views, "Starred", { type: "starred" });
  addRow(views, "Saved pages", { type: "saved" });
  addRow(views, "Continue reading", { type: "continue_reading" });

  const byCategory = new Map<string, typeof state.sources>();
  for (const s of state.sources) {
    const key = s.source.category ?? "";
    if (!byCategory.has(key)) byCategory.set(key, []);
    byCategory.get(key)!.push(s);
  }
  const categories = [...byCategory.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  for (const [cat, sources] of categories) {
    const g = addGroup(cat === "" ? "Feeds" : cat);
    for (const s of sources.sort((a, b) => a.source.title.localeCompare(b.source.title))) {
      addRow(g, s.source.title, { type: "source", id: s.source.id }, s.unread);
    }
  }

  if (state.tags.length > 0) {
    const g = addGroup("Tags");
    for (const t of state.tags.sort((a, b) => a.name.localeCompare(b.name))) {
      const row = document.createElement("div");
      row.className = "sidebar-row";
      const dot = document.createElement("span");
      dot.className = "tag-dot";
      dot.textContent = "#";
      const l = document.createElement("span");
      l.className = "label";
      l.textContent = t.name;
      row.append(dot, l);
      row.addEventListener("click", () => setFilter({ type: "tag", name: t.name }, `#${t.name}`));
      g.appendChild(row);
    }
  }
}

onSidebar(renderSidebar);

// Marking everything in the current filter as read.
export async function markAllReadCurrent(): Promise<void> {
  try {
    const count = await api.markAllRead({
      filter: state.filter,
      unread_only: false,
      keep: [],
      limit: 200,
      offset: 0,
    });
    toast(count > 0 ? `Marked ${count} read` : "Nothing unread here");
  } catch (e) {
    toast(String(e), true);
  }
}
