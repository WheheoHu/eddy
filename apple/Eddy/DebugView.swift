import EddyKit
import SwiftUI

/// Temporary developer console that exercises the engine API until the real
/// interface is designed. It intentionally has no visual design.
@Observable
@MainActor
final class DebugModel {
    var engine: EddyEngine?
    var status: StatusView?
    var items: [ItemView] = []
    var selected: ItemView.ID?
    var documentText = ""
    var message = ""
    var busy = false

    var url = ""
    var username = ""
    var password = ""
    var saveURL = ""
    var llmURL = ""
    var llmModel = ""
    var llmKey = ""
    private var spoolWatcher: SpoolWatcher?

    func saveAI() {
        guard let engine else { return }
        do {
            var settings = try engine.aiSettings()
            settings = AiSettingsView(
                provider: ProviderView(baseUrl: llmURL, model: llmModel),
                targetLanguage: settings.targetLanguage,
                glossary: settings.glossary,
                syncTranslations: settings.syncTranslations,
                syncSummaries: settings.syncSummaries,
                dailyAutoLimit: settings.dailyAutoLimit,
                rules: settings.rules
            )
            try engine.setAiSettings(settings: settings)
            if !llmKey.isEmpty { try engine.setAiKey(key: llmKey) }
            llmKey = ""
            message = "模型设置已保存"
        } catch {
            message = "保存模型设置失败：\(error.localizedDescription)"
        }
    }

    func ai(_ action: String) {
        guard let id = selected else { return }
        run(action) { engine in
            switch action {
            case "翻译":
                let t = try await engine.translate(itemId: id, force: false, listener: nil)
                return "译文 \(t.segments.count) 段\(t.cached ? "（缓存）" : "")：\(t.segments["title"] ?? "")"
            case "摘要":
                let s = try await engine.summarize(itemId: id, force: false)
                return "摘要：\(s.tldr) · " + s.points.joined(separator: " / ")
            default:
                let tags = try await engine.suggestTags(itemId: id)
                return "标签：\(tags.tags.map(\.name).joined(separator: ", ")) · 建议：\(tags.suggestions.joined(separator: ", "))"
            }
        }
    }

    init() {
        do {
            let engine = try EddyCore.openDefault()
            try engine.setWebRenderer(renderer: WebViewRenderer())
            self.engine = engine
            spoolWatcher = SpoolWatcher(path: engine.spoolDir()) { [weak self] in
                Task { @MainActor in self?.importCaptures() }
            }
            reload()
            importCaptures()
        } catch {
            message = "打开引擎失败：\(error.localizedDescription)"
        }
    }

    func importCaptures() {
        guard let engine else { return }
        Task {
            if let n = try? await engine.importCaptures(), n > 0 {
                message = "导入了 \(n) 个浏览器捕获的页面"
                reload()
            }
        }
    }

    func save() {
        let target = saveURL
        run("保存") { engine in
            let item = try await engine.saveUrl(url: target)
            return "已保存：\(item.title)"
        }
        saveURL = ""
    }

    func fetchFullText() {
        guard let id = selected else { return }
        run("抓全文") { [weak self] engine in
            let doc = try await engine.fetchFullText(itemId: id)
            await MainActor.run { self?.show(doc) }
            return "全文 \(doc.blocks.count) 块（\(doc.origin)）"
        }
    }

    func installChromeHost() {
        let helper = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/eddy-capture-host").path
        do {
            let files = try installNativeHost(hostPath: helper)
            message = "已连接 Chrome 扩展（ID \(chromeExtensionId())），写入 \(files.count) 个浏览器配置"
        } catch {
            message = "连接 Chrome 扩展失败：\(error.localizedDescription)"
        }
    }

    func show(_ doc: DocumentView) {
        documentText = "[\(doc.origin)]\n\n" + doc.blocks.map { "[\($0.id)] \($0.kind.plainText)" }.joined(separator: "\n\n")
    }

    func reload() {
        guard let engine else { return }
        status = try? engine.status()
        items = (try? engine.items(query: ItemQueryView(filter: .all, unreadOnly: false, limit: 300, offset: 0))) ?? []
    }

    func run(_ label: String, _ work: @escaping (EddyEngine) async throws -> String) {
        guard let engine, !busy else { return }
        busy = true
        message = "\(label)…"
        Task {
            do {
                message = try await work(engine)
            } catch {
                message = "\(label)失败：\(error.localizedDescription)"
            }
            busy = false
            reload()
        }
    }

    func connect() {
        let (u, n, p) = (url, username, password)
        run("连接") { engine in
            let account = try await engine.configureFreshrss(url: u, username: n, password: p)
            return "已连接 \(account.baseUrl)"
        }
        password = ""
    }

    func refresh() {
        run("刷新") { engine in
            let s = try await engine.refresh()
            return "新文章 \(s.newItems)，状态变化 \(s.stateChanges)，订阅 \(s.sources)"
        }
    }

    func open(_ id: String) {
        guard let engine else { return }
        Task {
            do {
                if let doc = try await engine.document(itemId: id) {
                    show(doc)
                } else {
                    documentText = "（没有内容）"
                }
                try await engine.setRead(itemId: id, read: true)
                reload()
            } catch {
                documentText = "读取失败：\(error.localizedDescription)"
            }
        }
    }
}

extension ItemView: Identifiable {}

struct DebugView: View {
    @Bindable var model: DebugModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("core \(EddyCore.version)")
                Text(statusLine).foregroundStyle(.secondary)
                Spacer()
                Button("刷新", action: model.refresh).disabled(model.busy)
            }
            HStack {
                TextField("FreshRSS 地址", text: $model.url)
                TextField("用户名", text: $model.username).frame(width: 140)
                SecureField("API 密码", text: $model.password).frame(width: 140)
                Button("连接", action: model.connect).disabled(model.busy)
            }
            HStack {
                TextField("粘贴网址稍后阅读", text: $model.saveURL).onSubmit(model.save)
                Button("保存", action: model.save).disabled(model.busy || model.saveURL.isEmpty)
                Button("抓全文", action: model.fetchFullText).disabled(model.busy || model.selected == nil)
                Button("连接 Chrome 扩展", action: model.installChromeHost)
            }
            HStack {
                TextField("模型 API 地址", text: $model.llmURL)
                TextField("模型 ID", text: $model.llmModel).frame(width: 160)
                SecureField("API Key", text: $model.llmKey).frame(width: 160)
                Button("保存模型", action: model.saveAI)
                ForEach(["翻译", "摘要", "标签"], id: \.self) { action in
                    Button(action) { model.ai(action) }.disabled(model.busy || model.selected == nil)
                }
            }
            Text(model.message).font(.caption).foregroundStyle(.secondary)
            HSplitView {
                List(model.items, selection: $model.selected) { item in
                    VStack(alignment: .leading) {
                        Text(item.title).fontWeight(item.read ? .regular : .bold)
                        Text(item.sourceTitle ?? "").font(.caption).foregroundStyle(.secondary)
                    }
                }
                .frame(minWidth: 280)
                ScrollView {
                    Text(model.documentText)
                        .font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding()
                }
            }
        }
        .padding()
        .onChange(of: model.selected) { _, id in
            if let id { model.open(id) }
        }
    }

    private var statusLine: String {
        guard let s = model.status else { return "" }
        let account = s.account.map { "\($0.username) @ \($0.baseUrl)" } ?? "未连接"
        return "\(s.mode) · \(account)\(s.lastError.map { " · \($0)" } ?? "")"
    }
}
