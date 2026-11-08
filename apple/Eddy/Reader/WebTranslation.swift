import EddyKit
import Observation
import WebKit

/// Translates a web page where it stands, the way the reflowed reader shows
/// a translation: each paragraph followed by its translation, or replaced
/// by it. The page's paragraphs are found by a script in the page, sent to
/// the engine in batches starting from the one on screen, and written back
/// as they arrive. Nothing is stored or synced: the text is the page's,
/// and goes with it when the page does.
@Observable
@MainActor
final class WebTranslator {
    private(set) var translating = false
    /// Stopped part-way, by a cancel or an error; `show` continues it.
    private(set) var stopped = false
    @ObservationIgnored weak var webView: WKWebView?

    @ObservationIgnored private let model: AppModel
    @ObservationIgnored private let key: String
    @ObservationIgnored private var mode: ReadingMode = .original
    /// Segments of the current page already translated.
    @ObservationIgnored private var done: Set<String> = []
    @ObservationIgnored private var run: Task<Void, Never>?
    @ObservationIgnored private var generation = 0

    init(model: AppModel, itemID: String) {
        self.model = model
        key = "web:\(itemID)"
    }

    /// Shows the page in `mode`: original hides the translations and stops
    /// translating; the others show what is translated and translate the
    /// rest.
    func show(_ mode: ReadingMode) {
        self.mode = mode
        Task { await setPageMode() }
        if mode == .original {
            cancel()
        } else if !translating {
            start()
        }
    }

    /// A page finished loading: its paragraphs are new. Scripted pages
    /// still rebuild their text just after loading, so the paragraphs are
    /// collected a moment later.
    func pageLoaded() {
        done = []
        cancel()
        guard mode != .original else { return }
        Task { await setPageMode() }
        start(after: .milliseconds(600))
    }

    /// Stops after the batch in flight; what is translated stays shown.
    func cancel() {
        guard translating else { return }
        model.cancelTranslation(key)
        generation += 1
        translating = false
        stopped = true
    }

    private func start(after delay: Duration = .zero) {
        guard let webView else { return }
        generation += 1
        let generation = generation
        translating = true
        stopped = false
        let previous = run
        run = Task {
            await previous?.value
            if delay > .zero { try? await Task.sleep(for: delay) }
            // Paragraphs the page added while the first ones were being
            // translated (a script filling in the article) get a second
            // pass; it stops when a pass finds nothing new.
            for _ in 0..<3 {
                guard self.generation == generation else { return }
                let segments = (await collect(from: webView)).filter { !done.contains($0.id) }
                guard !segments.isEmpty else { break }
                do {
                    let all = try await model.translateText(key, segments) { [weak self] batch in
                        guard let self, self.generation == generation else { return }
                        Task { await self.write(batch) }
                    }
                    // Not every batch reaches the page when the run is
                    // cancelled mid-way; the translation keeps what did.
                    guard self.generation == generation else { return }
                    done.formUnion(all.keys)
                    if all.count < segments.count {
                        translating = false
                        stopped = true
                        return
                    }
                } catch {
                    guard self.generation == generation else { return }
                    model.message = error.localizedDescription
                    translating = false
                    stopped = true
                    return
                }
            }
            if self.generation == generation { translating = false }
        }
    }

    private func collect(from webView: WKWebView) async -> [SegmentView] {
        // The async evaluate wants a value back, not `undefined`.
        guard let result = try? await webView.evaluateJavaScript("window.__eddy ? window.__eddy.collect() : []"),
              let list = result as? [[String: Any]] else { return [] }
        return list.compactMap { entry in
            guard let id = entry["id"] as? String, let text = entry["text"] as? String else { return nil }
            return SegmentView(id: id, text: text)
        }
    }

    private func write(_ batch: [String: String]) async {
        guard let webView, !batch.isEmpty,
              let data = try? JSONSerialization.data(withJSONObject: batch),
              let json = String(data: data, encoding: .utf8) else { return }
        done.formUnion(batch.keys)
        _ = try? await webView.evaluateJavaScript("window.__eddy && window.__eddy.apply(\(json)); 0")
    }

    private func setPageMode() async {
        let name = switch mode {
        case .original: "off"
        case .bilingual: "bilingual"
        case .translation: "only"
        }
        _ = try? await webView?.evaluateJavaScript("window.__eddy && window.__eddy.mode('\(name)'); 0")
    }

    /// Runs in every page: finds the paragraphs, writes translations in,
    /// switches how they show. Injected at the end of the document.
    static let script = WKUserScript(source: pageScript, injectionTime: .atDocumentEnd, forMainFrameOnly: true)

    private static let pageScript = #"""
    (function () {
      if (window.__eddy) return;
      const BLOCK_SEL = 'p, div, section, article, main, header, footer, aside, nav, address, center, ul, ol, li, dl, dd, dt, table, thead, tbody, tfoot, tr, td, th, blockquote, pre, h1, h2, h3, h4, h5, h6, figure, figcaption, form, fieldset, details, summary, hr';
      const BLOCK_TAGS = new Set(BLOCK_SEL.toUpperCase().split(', '));
      const SKIP = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'CODE', 'PRE', 'KBD', 'SAMP', 'TEXTAREA', 'INPUT', 'BUTTON', 'SELECT', 'OPTION', 'LABEL', 'NAV', 'SVG', 'MATH', 'IFRAME', 'VIDEO', 'AUDIO', 'CANVAS', 'OBJECT', 'EMBED', 'TIME']);
      const CONTROL_ROLES = new Set(['button', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'tab', 'option', 'navigation', 'menubar', 'menu', 'toolbar']);
      let next = 0;

      // The translation reads like the reader's: the page's own text,
      // dimmed the way a secondary label is. Alone, it shows in full.
      const style = document.createElement('style');
      style.textContent = [
        '.eddy-src{display:contents}',
        '.eddy-tr{display:block;margin-top:.35em;opacity:.6}',
        'html.eddy-only .eddy-src{display:none}',
        'html.eddy-only .eddy-tr{display:inline;margin:0;opacity:1}',
        'html.eddy-off .eddy-tr{display:none}',
      ].join('\n');
      document.documentElement.appendChild(style);

      function skipped(el) {
        if (SKIP.has(el.tagName) || el.classList.contains('eddy-tr')) return true;
        if (el.hidden || el.getAttribute('aria-hidden') === 'true' || el.getAttribute('translate') === 'no') return true;
        if (el.isContentEditable || CONTROL_ROLES.has(el.getAttribute('role') || '')) return true;
        return false;
      }

      // Rows of links and lone link labels are the page's chrome (a
      // comment's "user · 2 hours ago | next", "reply", "login"), not its
      // prose: the model has nothing to translate and often trips on them.
      function chrome(el, text) {
        const links = el.querySelectorAll('a');
        if (!links.length) return false;
        let linked = 0, longest = 0;
        links.forEach(a => {
          const t = a.textContent.replace(/\s+/g, ' ').trim();
          linked += t.length;
          longest = Math.max(longest, t.split(' ').length);
        });
        // A row of short links; a lone short label. A linked headline
        // (several words) stays.
        if (longest >= 4) return false;
        if (links.length >= 2) return linked >= text.length * 0.7;
        return linked >= text.length * 0.9 && text.split(' ').length < 3;
      }

      // A child that is a block, or holds one, ends a paragraph.
      function boundary(node) {
        return node.nodeType === 1 && (BLOCK_TAGS.has(node.tagName) || node.classList.contains('eddy-tr') || node.querySelector(BLOCK_SEL) !== null);
      }

      // Walks down to the paragraphs: an element with no blocks inside is
      // one; in an element that holds blocks, each stretch of text and
      // inline elements between them (a comment whose first paragraph is
      // bare text before its <p>s) is wrapped so it can be one too.
      function visit(el, found) {
        if (el.nodeType !== 1 || skipped(el)) return;
        if (el.hasAttribute('data-eddy-seg')) { found.push(el); return; }
        const nodes = Array.from(el.childNodes);
        if (!nodes.some(boundary)) { found.push(el); return; }
        let run = [];
        const flush = () => {
          if (!run.length) return;
          const text = run.map(n => n.textContent).join('').trim();
          if (text.length >= 2) {
            const wrap = document.createElement('span');
            wrap.style.display = 'contents';
            el.insertBefore(wrap, run[0]);
            run.forEach(n => wrap.appendChild(n));
            found.push(wrap);
          }
          run = [];
        };
        for (const node of nodes) {
          if (boundary(node)) { flush(); visit(node, found); } else { run.push(node); }
        }
        flush();
      }

      function textOf(el) {
        return (el.innerText || el.textContent || '').replace(/\s+/g, ' ').trim();
      }

      function worth(text) {
        return text.length >= 2 && /\p{L}/u.test(text);
      }

      function collect() {
        const found = [];
        visit(document.body, found);
        const segments = [];
        for (const el of found) {
          let r = el.getBoundingClientRect();
          if (el.style.display === 'contents') {
            // A wrapper has no box of its own; its contents do.
            const range = document.createRange();
            range.selectNodeContents(el);
            r = range.getBoundingClientRect();
          }
          if (r.width === 0 && r.height === 0) continue;
          const text = textOf(el);
          if (!worth(text) || chrome(el, text)) continue;
          let id = el.getAttribute('data-eddy-seg');
          if (!id) { id = 's' + (next++); el.setAttribute('data-eddy-seg', id); }
          segments.push({ id, text, top: r.top });
        }
        // From the first on screen on down, then the ones above.
        const first = segments.findIndex(s => s.top >= -8);
        const ordered = first > 0 ? segments.slice(first).concat(segments.slice(0, first)) : segments;
        return ordered.map(s => ({ id: s.id, text: s.text }));
      }

      function apply(map) {
        for (const id in map) {
          const el = document.querySelector('[data-eddy-seg="' + id + '"]');
          if (!el || el.querySelector(':scope > .eddy-tr')) continue;
          if (!el.querySelector(':scope > .eddy-src')) {
            const src = document.createElement('span');
            src.className = 'eddy-src';
            while (el.firstChild) src.appendChild(el.firstChild);
            el.appendChild(src);
          }
          const tr = document.createElement('span');
          tr.className = 'eddy-tr';
          tr.textContent = map[id];
          el.appendChild(tr);
        }
      }

      function mode(name) {
        const c = document.documentElement.classList;
        c.remove('eddy-only', 'eddy-off');
        if (name === 'only') c.add('eddy-only');
        if (name === 'off') c.add('eddy-off');
      }

      window.__eddy = { collect, apply, mode };
    })();
    """#
}
