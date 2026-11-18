// Typed wrappers over the Tauri commands, mirroring the view structs in
// src-tauri/src/views.rs (snake_case throughout).

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Item {
  id: string;
  source_id: string | null;
  kind: "feed" | "saved" | "chapter";
  url: string | null;
  title: string;
  author: string | null;
  published_at: number;
  read: boolean;
  starred: boolean;
  series_id: string | null;
  chapter_index: number | null;
  chapter_level: number;
  excerpt: string | null;
  lang: string | null;
  text_len: number;
  progress: number;
  finished: boolean;
}

export interface ItemRow {
  item: Item;
  source_title: string | null;
  format: "article" | "pdf";
}

export interface Account {
  id: string;
  kind: string;
  base_url: string;
  username: string;
  created_at: number;
}

export interface Status {
  mode: "unconfigured" | "standalone" | "server";
  account: Account | null;
  last_refresh: number | null;
  last_error: string | null;
  server_online: boolean;
  pending_uploads: number;
  rejected_uploads: number;
  offline_refresh: boolean;
  sync: { phase: string; done: number; total: number | null } | null;
}

export interface Source {
  id: string;
  kind: "feed" | "saved";
  title: string;
  category: string | null;
  site_url: string | null;
}

export interface SourceView {
  source: Source;
  unread: number;
}

export interface TagView {
  id: string;
  name: string;
}

export type Filter =
  | { type: "all" }
  | { type: "starred" }
  | { type: "saved" }
  | { type: "continue_reading" }
  | { type: "source"; id: string }
  | { type: "category"; name: string }
  | { type: "series"; id: string }
  | { type: "tag"; name: string }
  | { type: "title"; text: string };

export interface Query {
  filter: Filter;
  unread_only: boolean;
  keep: string[];
  limit: number;
  offset: number;
}

export function query(filter: Filter, extra?: Partial<Query>): Query {
  return { filter, unread_only: false, keep: [], limit: 200, offset: 0, ...extra };
}

export interface Run {
  text: string;
  style: number;
  link: string | null;
  math: boolean;
}

export interface HighlightSpan {
  start: number;
  end: number;
  class: string;
}

export type BlockKind =
  | { type: "heading"; level: number; runs: Run[] }
  | { type: "paragraph"; runs: Run[] }
  | { type: "list_item"; ordered: boolean; depth: number; index: number; marker: boolean; runs: Run[] }
  | { type: "code"; language: string | null; text: string; highlights: HighlightSpan[] }
  | { type: "math"; tex: string }
  | { type: "image"; src: string; alt: string; width: number | null; height: number | null; caption: Run[] }
  | { type: "table"; rows: { cells: { header: boolean; colspan: number; rowspan: number; runs: Run[] }[] }[] }
  | { type: "media"; media: "video" | "audio" | "embed"; src: string; poster: string | null; title: string | null }
  | { type: "rule" };

export interface Block {
  id: string;
  quote_depth: number;
  kind: BlockKind;
}

export interface Doc {
  schema_version: number;
  blocks: Block[];
  pages: string[];
}

export interface DocumentView {
  version: { hash: string; origin: string; source_url: string | null };
  document: Doc;
}

export interface SearchResultRow {
  item: ItemRow;
  snippet: string;
  highlights: [number, number][];
}

export interface RefreshSummary {
  new_items: number;
  state_changes: number;
  sources: number;
  new_item_ids: string[];
  offline: boolean;
}

export interface Imported {
  item_id: string;
  series_id: string | null;
  title: string;
  existing: boolean;
}

export interface ServerStatusView {
  url: string;
  server_id: string;
  cursor: number;
  video_capable: boolean;
  version: string;
}

export interface ResumePoint {
  block_id: string | null;
  offset: number;
  page: number | null;
  progression: number;
  from_other_device: boolean;
  local_block_id: string | null;
  local_offset: number;
}

export interface ProviderSettings {
  protocol: "auto" | "openai" | "gemini";
  base_url: string;
  model: string;
  max_concurrency: number;
}

export interface AiSettings {
  translation_provider: ProviderSettings | null;
  target_language: string;
  glossary: string;
  sync_translations: boolean;
  translation_daily_limit: number;
  summary_provider: ProviderSettings | null;
  summary_language: string;
  sync_summaries: boolean;
  summary_daily_limit: number;
  rules: unknown[];
}

export interface TranslationView {
  content_hash: string;
  target_language: string;
  segments: Record<string, string>;
  cached: boolean;
  complete: boolean;
}

export interface TranslationBatch {
  item_id: string;
  segments: Record<string, string>;
}

export interface SummaryView {
  tldr: string;
  points: string[];
  cached: boolean;
}

// ---------- invoke wrappers ----------

export const api = {
  status: (): Promise<Status> => invoke("status"),
  pdf_available: (): Promise<boolean> => invoke("pdf_available"),
  sources: (): Promise<SourceView[]> => invoke("sources"),
  tags: (): Promise<TagView[]> => invoke("tags"),
  items: (q: Query): Promise<ItemRow[]> => invoke("items", { query: q }),
  item: (id: string): Promise<ItemRow | null> => invoke("item", { id }),
  itemCount: (q: Query): Promise<number> => invoke("item_count", { query: q }),
  document: (id: string): Promise<DocumentView | null> => invoke("document", { id }),
  fetchFullText: (id: string): Promise<DocumentView> => invoke("fetch_full_text", { id }),
  applyRules: (id: string): Promise<void> => invoke("apply_rules", { id }),
  image: (url: string): Promise<string> => invoke("image", { url }),
  setRead: (id: string, read: boolean): Promise<void> => invoke("set_read", { id, read }),
  setStarred: (id: string, starred: boolean): Promise<void> => invoke("set_starred", { id, starred }),
  markAllRead: (q: Query): Promise<number> => invoke("mark_all_read", { query: q }),
  resumePoint: (id: string): Promise<ResumePoint | null> => invoke("resume_point", { id }),
  recordProgress: (id: string, blockId: string, offset: number, atEnd: boolean): Promise<void> =>
    invoke("record_progress", { id, blockId, offset, atEnd }),
  saveUrl: (url: string): Promise<ItemRow> => invoke("save_url", { url }),
  importFile: (path: string): Promise<Imported> => invoke("import_file", { path }),
  searchAll: (query: string, limit: number): Promise<SearchResultRow[]> =>
    invoke("search_all", { query, limit }),
  refresh: (): Promise<RefreshSummary> => invoke("refresh"),
  configureFreshrss: (url: string, username: string, password: string): Promise<Account> =>
    invoke("configure_freshrss", { url, username, password }),
  removeAccount: (): Promise<void> => invoke("remove_account"),
  connectServer: (url: string, code: string): Promise<ServerStatusView> =>
    invoke("connect_server", { url, code }),
  disconnectServer: (): Promise<void> => invoke("disconnect_server"),
  serverStatus: (): Promise<ServerStatusView | null> => invoke("server_status"),
  aiSettings: (): Promise<AiSettings> => invoke("ai_settings"),
  setAiSettings: (settings: AiSettings): Promise<void> => invoke("set_ai_settings", { settings }),
  setAiKey: (role: string, key: string | null): Promise<void> => invoke("set_ai_key", { role, key }),
  hasAiKey: (role: string): Promise<boolean> => invoke("has_ai_key", { role }),
  translate: (itemId: string, force: boolean): Promise<TranslationView> =>
    invoke("translate", { itemId, force }),
  cancelTranslation: (itemId: string): Promise<void> => invoke("cancel_translation", { itemId }),
  cachedTranslation: (itemId: string): Promise<TranslationView | null> =>
    invoke("cached_translation", { itemId }),
  summarize: (itemId: string, force: boolean): Promise<SummaryView> =>
    invoke("summarize", { itemId, force }),
  cachedSummary: (itemId: string): Promise<SummaryView | null> =>
    invoke("cached_summary", { itemId }),
  openExternal: (url: string): Promise<void> => invoke("open_external", { url }),
  cacheUsage: (): Promise<number> => invoke("cache_usage"),
  setCacheLimit: (bytes: number): Promise<void> => invoke("set_cache_limit", { bytes }),
};

export function onEvent(name: string, handler: () => void): Promise<UnlistenFn> {
  return listen(name, handler);
}

export function onTranslation(handler: (batch: TranslationBatch) => void): Promise<UnlistenFn> {
  return listen<TranslationBatch>("eddy:translation", (e) => handler(e.payload));
}
