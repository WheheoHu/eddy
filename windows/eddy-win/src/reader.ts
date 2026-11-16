import { api, type Block, type BlockKind, type Run, type DocumentView } from "./api";
import { state, toast, fmtDate } from "./state";
import { patchItem, loadItems, renderList } from "./list";

// ---------- style bits (mirror eddy-model/src/doc.rs) ----------

const BOLD = 1, ITALIC = 2, CODE = 4, STRIKE = 8, UNDERLINE = 16,
  SUPERSCRIPT = 32, SUBSCRIPT = 64, SMALL = 128, MARK = 256;

function runSpan(run: Run, onLink?: (url: string) => void): HTMLElement {
  if (run.math) {
    const el = document.createElement("span");
    el.className = "math-run";
    el.textContent = `$${run.text}$`;
    el.title = run.text;
    return el;
  }
  let el: HTMLElement = document.createElement("span");
  el.textContent = run.text;
  if (run.style & BOLD) {
    const b = document.createElement("b");
    b.append(el.firstChild!);
    el.append(b);
  }
  if (run.style & ITALIC) {
    const i = document.createElement("i");
    i.append(el.firstChild!);
    el.append(i);
  }
  if (run.style & CODE) {
    const c = document.createElement("code");
    c.className = "run-code";
    c.append(el.firstChild!);
    el.append(c);
  }
  if (run.style & STRIKE) {
    const s = document.createElement("s");
    s.append(el.firstChild!);
    el.append(s);
  }
  if (run.style & UNDERLINE) {
    const u = document.createElement("u");
    u.append(el.firstChild!);
    el.append(u);
  }
  if (run.style & SUPERSCRIPT) {
    const sup = document.createElement("sup");
    sup.append(el.firstChild!);
    el.append(sup);
  }
  if (run.style & SUBSCRIPT) {
    const sub = document.createElement("sub");
    sub.append(el.firstChild!);
    el.append(sub);
  }
  if (run.style & SMALL) el.style.fontSize = "0.85em";
  if (run.style & MARK) el.style.background = "#fff3bf";
  if (run.link) {
    const a = document.createElement("a");
    a.href = "#";
    a.append(el);
    a.addEventListener("click", (ev) => {
      ev.preventDefault();
      if (onLink) onLink(run.link!);
    });
    el = a;
  }
  return el;
}

function runsToFragment(runs: Run[], onLink?: (url: string) => void): DocumentFragment {
  const frag = document.createDocumentFragment();
  for (const run of runs) frag.append(runSpan(run, onLink));
  return frag;
}

// ---------- code highlighting (UTF-16 offsets === JS string indices) ----------

function highlightCode(text: string, highlights: { start: number; end: number; class: string }[]): DocumentFragment {
  const frag = document.createDocumentFragment();
  let pos = 0;
  for (const h of highlights) {
    if (h.start > pos) frag.append(text.slice(pos, h.start));
    if (h.end > h.start) {
      const span = document.createElement("span");
      span.className = `tok-${h.class.replace("_", "-")}`;
      span.textContent = text.slice(h.start, h.end);
      frag.append(span);
      pos = h.end;
    }
  }
  if (pos < text.length) frag.append(text.slice(pos));
  return frag;
}

// ---------- images ----------

const imageCache = new Map<string, Promise<string>>();

function imageUrl(src: string): Promise<string> {
  let p = imageCache.get(src);
  if (!p) {
    p = api.image(src);
    imageCache.set(src, p);
  }
  return p;
}

function observeImages(root: HTMLElement): void {
  const imgs = Array.from(root.querySelectorAll("img[data-src]")) as HTMLImageElement[];
  const io = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const img = entry.target as HTMLImageElement;
      io.unobserve(img);
      void imageUrl(img.dataset.src!)
        .then((url) => { img.src = url; })
        .catch(() => {
          img.replaceWith(Object.assign(document.createElement("span"), { textContent: "[image unavailable]" }));
        });
    }
  }, { rootMargin: "600px" });
  imgs.forEach((img) => io.observe(img));
}

// ---------- block rendering ----------

function renderBlock(block: Block, body: HTMLElement, onLink?: (url: string) => void): HTMLElement | null {
  const kind: BlockKind = block.kind;
  let el: HTMLElement;
  switch (kind.type) {
    case "heading": {
      const level = Math.min(Math.max(kind.level + 1, 2), 6);
      el = document.createElement(`h${level}`);
      el.append(runsToFragment(kind.runs, onLink));
      break;
    }
    case "paragraph": {
      el = document.createElement("p");
      el.append(runsToFragment(kind.runs, onLink));
      break;
    }
    case "list_item": {
      el = document.createElement("div");
      el.className = "li";
      el.style.marginLeft = `${kind.depth * 22}px`;
      const marker = document.createElement("span");
      marker.className = "marker";
      marker.textContent = kind.ordered ? `${kind.index}.` : "•";
      el.append(marker);
      el.append(runsToFragment(kind.runs, onLink));
      break;
    }
    case "code": {
      el = document.createElement("pre");
      const code = document.createElement("code");
      if (kind.language) code.dataset.lang = kind.language;
      code.append(highlightCode(kind.text, kind.highlights));
      el.append(code);
      break;
    }
    case "math": {
      el = document.createElement("div");
      el.className = "math-block";
      el.textContent = kind.tex;
      el.title = "LaTeX (rendering coming later)";
      break;
    }
    case "image": {
      el = document.createElement("figure");
      const img = document.createElement("img");
      img.dataset.src = kind.src;
      img.alt = kind.alt;
      if (kind.width && kind.height) {
        img.width = Math.min(kind.width, 720);
      }
      el.append(img);
      if (kind.caption.length > 0) {
        const cap = document.createElement("figcaption");
        cap.append(runsToFragment(kind.caption, onLink));
        el.append(cap);
      }
      break;
    }
    case "table": {
      el = document.createElement("table");
      for (const row of kind.rows) {
        const tr = document.createElement("tr");
        for (const cell of row.cells) {
          const td = document.createElement(cell.header ? "th" : "td");
          if (cell.colspan > 1) td.colSpan = cell.colspan;
          if (cell.rowspan > 1) td.rowSpan = cell.rowspan;
          td.append(runsToFragment(cell.runs, onLink));
          tr.append(td);
        }
        el.append(tr);
      }
      break;
    }
    case "media": {
      el = document.createElement("p");
      const a = document.createElement("a");
      a.href = "#";
      a.textContent = `[${kind.media}] ${kind.title ?? kind.src}`;
      a.addEventListener("click", (ev) => {
        ev.preventDefault();
        if (onLink) onLink(kind.src);
      });
      el.append(a);
      break;
    }
    case "rule": {
      el = document.createElement("hr");
      break;
    }
  }
  el.id = `blk-${CSS.escape(block.id)}`;
  body.appendChild(el);
  return el;
}

// Consecutive blocks with the same quote_depth > 0 collapse into blockquotes.
function renderDocument(doc: DocumentView, body: HTMLElement, onLink?: (url: string) => void): void {
  let quoteOpen = false;
  const closeQuote = () => { quoteOpen = false; };
  for (const block of doc.document.blocks) {
    if (block.quote_depth > 0) {
      let bq: HTMLElement;
      if (!quoteOpen) {
        bq = document.createElement("blockquote");
        bq.id = `blk-${CSS.escape(block.id)}`;
        body.appendChild(bq);
        quoteOpen = true;
      } else {
        bq = body.lastElementChild as HTMLElement;
      }
      const kind = block.kind;
      if (kind.type === "paragraph") bq.append(runsToFragment(kind.runs, onLink));
      else if (kind.type === "heading") {
        const h = document.createElement("h4");
        h.append(runsToFragment(kind.runs, onLink));
        bq.append(h);
      } else if (kind.type === "list_item") {
        const p = document.createElement("p");
        p.append(runsToFragment(kind.runs, onLink));
        bq.append(p);
      }
      continue;
    }
    quoteOpen = false;
    renderBlock(block, body, onLink);
  }
  void closeQuote;
}

// ---------- reading progress ----------

let progressItem: string | null = null;

function reportProgress(atEnd: boolean): void {
  if (!progressItem || !state.doc) return;
  const readerEl = document.getElementById("reader")!;
  const rect = readerEl.getBoundingClientRect();
  let best: string | null = null;
  for (const child of Array.from(document.querySelectorAll("#reader-body > [id^=blk-], #reader-body > blockquote[id^=blk-]"))) {
    const r = child.getBoundingClientRect();
    if (r.bottom > rect.top + 60) {
      best = (child as HTMLElement).id.slice(4);
      break;
    }
  }
  if (!best) return;
  const id = CSS.escape(best).replaceAll("\\", "");
  void api.recordProgress(progressItem, id, 0, atEnd).catch(() => {});
}

// ---------- open item ----------

/// Shown while no article is open: what this app does and where content
/// comes from, so a fresh profile does not look broken.
export function showReaderPlaceholder(): void {
  if (state.currentItemId) return;
  const readerEl = document.getElementById("reader")!;
  const el = document.createElement("div");
  el.className = "empty-reader";
  const configured = state.status && state.status.mode !== "unconfigured";
  el.innerHTML = `
    <div class="big">🌀</div>
    <div><b>Nothing open</b></div>
    <p>${configured
      ? "Pick an article on the left, or paste a URL in the toolbar and press Save."
      : "This library is empty. Pick one to start:"}</p>
    ${configured ? "" : `
    <p style="text-align:left; max-width:340px; margin:14px auto 0; line-height:1.8">
      🌐&nbsp; Paste a page URL in the toolbar above → <b>Save</b><br>
      📚&nbsp; <b>Import…</b> an EPUB or PDF file<br>
      📡&nbsp; <b>Settings → FreshRSS</b> to subscribe to your feeds
    </p>`}
  `;
  readerEl.textContent = "";
  readerEl.append(el);
}

export async function openItem(id: string): Promise<void> {
  state.currentItemId = id;
  state.doc = null;
  progressItem = null;
  renderList();

  const readerEl = document.getElementById("reader")!;
  readerEl.textContent = "";
  readerEl.scrollTop = 0;

  let row = state.rows.find((r) => r.item.id === id)?.item ?? null;
  if (!row) {
    const fetched = await api.item(id).catch(() => null);
    row = fetched?.item ?? null;
  }
  if (!row) {
    toast("Item not found", true);
    return;
  }

  // Header
  const header = document.createElement("div");
  header.className = "reader-header";
  const h1 = document.createElement("h1");
  h1.textContent = row.title || "(untitled)";
  const meta = document.createElement("div");
  meta.className = "meta";
  const metaParts = [state.rows.find((r) => r.item.id === id)?.source_title ?? "Saved"];
  if (row.author) metaParts.push(row.author);
  metaParts.push(fmtDate(row.published_at));
  meta.textContent = metaParts.join(" · ");
  const actions = document.createElement("div");
  actions.className = "reader-actions";

  const starBtn = document.createElement("button");
  starBtn.textContent = row.starred ? "★ Starred" : "☆ Star";
  starBtn.addEventListener("click", async () => {
    await api.setStarred(id, !row!.starred).catch((e) => toast(String(e), true));
    patchItem(id, { starred: !row!.starred });
    starBtn.textContent = row!.starred ? "★ Starred" : "☆ Star";
  });

  const unreadBtn = document.createElement("button");
  unreadBtn.textContent = row.read ? "Mark unread" : "Mark read";
  unreadBtn.addEventListener("click", async () => {
    await api.setRead(id, !row!.read).catch((e) => toast(String(e), true));
    patchItem(id, { read: !row!.read });
    unreadBtn.textContent = row!.read ? "Mark unread" : "Mark read";
    void loadItems();
  });

  const browserBtn = document.createElement("button");
  browserBtn.textContent = "Open in browser";
  browserBtn.addEventListener("click", () => {
    if (row!.url) void api.openExternal(row!.url).catch((e) => toast(String(e), true));
  });

  const fullBtn = document.createElement("button");
  fullBtn.textContent = "Fetch full text";
  fullBtn.addEventListener("click", async () => {
    fullBtn.disabled = true;
    fullBtn.textContent = "Fetching…";
    try {
      const doc = await api.fetchFullText(id);
      state.doc = doc;
      renderBody(doc, row!);
      toast("Full text fetched");
    } catch (e) {
      toast(String(e), true);
    }
    fullBtn.disabled = false;
    fullBtn.textContent = "Fetch full text";
  });

  const translateBtn = document.createElement("button");
  translateBtn.textContent = "Translate";
  translateBtn.addEventListener("click", () => void toggleTranslation(id, translateBtn));

  const summaryBtn = document.createElement("button");
  summaryBtn.textContent = "Summary";
  summaryBtn.addEventListener("click", () => void showSummary(id, summaryBtn));

  actions.append(starBtn, unreadBtn, browserBtn, fullBtn, translateBtn, summaryBtn);
  header.append(h1, meta, actions);
  readerEl.append(header);

  const bodyEl = document.createElement("div");
  bodyEl.id = "reader-body";
  readerEl.append(bodyEl);

  // Document (with rules applied on open, like the macOS reader)
  try {
    await api.applyRules(id);
  } catch { /* rules are best-effort */ }
  const doc = await api.document(id).catch((e) => { toast(String(e), true); return null; });
  if (!doc) {
    const empty = document.createElement("div");
    empty.className = "empty-reader";
    empty.textContent = "Content unavailable.";
    bodyEl.append(empty);
    return;
  }
  state.doc = doc;
  renderBody(doc, row);

  // Read state + progress
  if (!row.read) {
    await api.setRead(id, true).catch(() => {});
    patchItem(id, { read: true });
    void loadItems();
  }
  progressItem = id;

  readerEl.onscroll = () => {
    const atEnd = readerEl.scrollTop + readerEl.clientHeight >= readerEl.scrollHeight - 60;
    window.clearTimeout((reportProgress as unknown as { t?: number }).t);
    (reportProgress as unknown as { t?: number }).t = window.setTimeout(() => reportProgress(atEnd), 2000);
  };

  // Resume where the reader left off
  try {
    const resume = await api.resumePoint(id);
    if (resume && resume.block_id && resume.progression > 0.005 && resume.progression < 0.99) {
      const target = document.getElementById(`blk-${CSS.escape(resume.block_id)}`);
      if (target) {
        target.scrollIntoView({ block: "start" });
        toast("Resumed where you left off");
      }
    }
  } catch { /* resume is best-effort */ }
}

function renderBody(doc: DocumentView, row: { title: string }): void {
  const bodyEl = document.getElementById("reader-body")!;
  bodyEl.textContent = "";
  renderDocument(doc, bodyEl, (url) => void api.openExternal(url).catch((e) => toast(String(e), true)));
  observeImages(bodyEl);
  void row;
}

// ---------- AI: translation + summary ----------

async function toggleTranslation(id: string, btn: HTMLButtonElement): Promise<void> {
  if (btn.dataset.active === "1") {
    await api.cancelTranslation(id).catch(() => {});
    document.querySelectorAll(".translation").forEach((el) => el.remove());
    btn.dataset.active = "0";
    btn.textContent = "Translate";
    return;
  }
  const settings = await api.aiSettings().catch(() => null);
  if (!settings?.translation_provider) {
    toast("Add a translation provider in Settings first", true);
    return;
  }
  const cached = await api.cachedTranslation(id).catch(() => null);
  btn.dataset.active = "1";
  btn.textContent = "Translating…";
  if (cached && cached.complete) {
    applyTranslation(cached.segments);
    btn.textContent = "Hide translation";
    return;
  }
  try {
    const result = await api.translate(id, false);
    applyTranslation(result.segments);
    btn.textContent = "Hide translation";
  } catch (e) {
    toast(String(e), true);
    btn.dataset.active = "0";
    btn.textContent = "Translate";
  }
}

export function applyTranslation(segments: Record<string, string>): void {
  for (const [segId, text] of Object.entries(segments)) {
    if (!segId.startsWith("#")) insertTranslation(segId, text);
  }
}

function insertTranslation(blockId: string, text: string): void {
  const anchor = document.getElementById(`blk-${CSS.escape(blockId)}`);
  if (!anchor || anchor.nextElementSibling?.classList.contains("translation")) return;
  const el = document.createElement("div");
  el.className = "translation";
  el.textContent = text;
  anchor.after(el);
}

async function showSummary(id: string, btn: HTMLButtonElement): Promise<void> {
  const panel = document.getElementById("summary-panel")!;
  const body = document.getElementById("summary-body")!;
  panel.hidden = false;
  body.textContent = "Summarizing…";
  try {
    let s = await api.cachedSummary(id).catch(() => null);
    if (!s) s = await api.summarize(id, false);
    body.textContent = "";
    const tldr = document.createElement("div");
    tldr.className = "tldr";
    tldr.textContent = s.tldr;
    body.append(tldr);
    const ul = document.createElement("ul");
    for (const p of s.points) {
      const li = document.createElement("li");
      li.textContent = p;
      ul.append(li);
    }
    body.append(ul);
    void btn;
  } catch (e) {
    body.textContent = String(e);
  }
}
