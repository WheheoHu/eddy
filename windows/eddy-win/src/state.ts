import { api, type Status, type SourceView, type TagView, type ItemRow, type Filter, type DocumentView } from "./api";

// App-wide snapshot state, reloaded in full whenever the engine pulses
// "eddy:change" — the same shape as the macOS app's AppModel.

export interface AppState {
  status: Status | null;
  sources: SourceView[];
  tags: TagView[];
  rows: ItemRow[];
  filter: Filter;
  filterLabel: string;
  unreadOnly: boolean;
  currentItemId: string | null;
  doc: DocumentView | null;
}

export const state: AppState = {
  status: null,
  sources: [],
  tags: [],
  rows: [],
  filter: { type: "all" },
  filterLabel: "All",
  unreadOnly: false,
  currentItemId: null,
  doc: null,
};

type Listener = () => void;
const sidebarListeners: Listener[] = [];
const listListeners: Listener[] = [];

export function onSidebar(fn: Listener) { sidebarListeners.push(fn); }
export function onList(fn: Listener) { listListeners.push(fn); }

export function emitSidebar() { for (const fn of sidebarListeners) fn(); }
export function emitList() { for (const fn of listListeners) fn(); }

export async function reloadSnapshot(): Promise<void> {
  const [status, sources, tags] = await Promise.all([
    api.status().catch(() => null),
    api.sources().catch(() => []),
    api.tags().catch(() => []),
  ]);
  state.status = status;
  state.sources = sources;
  state.tags = tags;
  emitSidebar();
}

export function toast(message: string, error = false): void {
  const el = document.getElementById("toast")!;
  el.textContent = message;
  el.classList.toggle("error", error);
  el.hidden = false;
  window.clearTimeout((toast as unknown as { t?: number }).t);
  (toast as unknown as { t?: number }).t = window.setTimeout(() => {
    el.hidden = true;
  }, error ? 5000 : 2500);
}

export function fmtDate(ms: number): string {
  const d = new Date(ms);
  const today = new Date();
  if (d.toDateString() === today.toDateString()) {
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  return d.toLocaleDateString([], { month: "short", day: "numeric" });
}
