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

    init() {
        do {
            engine = try EddyCore.openDefault()
            reload()
        } catch {
            message = "打开引擎失败：\(error.localizedDescription)"
        }
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
                    documentText = doc.blocks.map { "[\($0.id)] \($0.kind.plainText)" }.joined(separator: "\n\n")
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
