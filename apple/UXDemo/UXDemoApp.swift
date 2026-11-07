import EddyKit
import SwiftUI

enum Direction: String, CaseIterable, Identifiable {
    case classic = "A · 经典三栏"
    case shelf = "B · 阅读优先"
    case magazine = "C · 信息流"
    var id: String { rawValue }
}

@main
struct UXDemoApp: App {
    @State private var store: DemoStore
    @AppStorage("direction") private var direction: Direction = .classic

    init() {
        DemoStore.applyPendingReset()
        _store = State(initialValue: DemoStore())
    }

    var body: some Scene {
        WindowGroup("Eddy UX Demo") {
            DemoRoot(direction: $direction)
                .environment(store)
                .frame(minWidth: 1100, minHeight: 700)
        }
        .defaultSize(width: 1400, height: 880)
        .commands {
            CommandMenu("方向") {
                ForEach(Array(Direction.allCases.enumerated()), id: \.element) { index, d in
                    Button(d.rawValue) { direction = d }.keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: .command)
                }
            }
        }
        Settings {
            DemoSettings().environment(store)
        }
    }
}

struct DemoRoot: View {
    @Environment(DemoStore.self) private var store
    @Binding var direction: Direction

    var body: some View {
        Group {
            switch direction {
            case .classic: ClassicView()
            case .shelf: ShelfView()
            case .magazine: MagazineView()
            }
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                Picker("方向", selection: $direction) {
                    ForEach(Direction.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .frame(width: 360)
            }
        }
        .overlay(alignment: .bottom) {
            if let message = store.message {
                Banner(icon: "info.circle", text: message) {
                    Button("好") { store.message = nil }
                }
                .padding(20)
                .task(id: message) {
                    try? await Task.sleep(for: .seconds(6))
                    if store.message == message { store.message = nil }
                }
            }
        }
    }
}

struct DemoSettings: View {
    @Environment(DemoStore.self) private var store
    @State private var freshURL = ""
    @State private var freshUser = ""
    @State private var freshPassword = ""
    @State private var llmURL = ""
    @State private var llmModel = ""
    @State private var llmKey = ""
    @State private var target = "简体中文"
    @State private var note = ""

    var body: some View {
        TabView {
            Form {
                if let account = store.status?.account {
                    LabeledContent("已连接", value: "\(account.username) @ \(account.baseUrl)")
                }
                TextField("FreshRSS 地址", text: $freshURL)
                TextField("用户名", text: $freshUser)
                SecureField("API 专用密码", text: $freshPassword)
                Button("连接并刷新") { connect() }
                Text(note).foregroundStyle(.secondary)
            }
            .tabItem { Label("订阅", systemImage: "dot.radiowaves.up.forward") }
            Form {
                TextField("模型 API 地址", text: $llmURL)
                TextField("模型 ID", text: $llmModel)
                SecureField("API Key（只存在这台 Mac 的钥匙串）", text: $llmKey)
                TextField("目标语言", text: $target)
                Button("保存") { saveAI() }
                Text(note).foregroundStyle(.secondary)
            }
            .tabItem { Label("AI", systemImage: "sparkles") }
            Form {
                LabeledContent("数据目录", value: DemoStore.dataDirectory.path)
                Text("demo 使用独立的数据目录和钥匙串条目，和正式版 Eddy 互不影响。")
                Button("下次启动时重新准备示例内容") { DemoStore.requestReset(); note = "退出并重新打开 demo 后生效" }
                Text(note).foregroundStyle(.secondary)
            }
            .tabItem { Label("Demo", systemImage: "wand.and.stars") }
        }
        .padding(20)
        .frame(width: 520, height: 320)
        .onAppear {
            if let p = try? store.engine?.aiSettings() {
                llmURL = p.provider?.baseUrl ?? ""
                llmModel = p.provider?.model ?? ""
                target = p.targetLanguage
            }
        }
    }

    private func connect() {
        let (u, n, p) = (freshURL, freshUser, freshPassword)
        freshPassword = ""
        note = "连接中…"
        Task {
            do {
                _ = try await store.engine?.configureFreshrss(url: u, username: n, password: p)
                note = "已连接，正在刷新"
                await store.refresh()
                note = store.message ?? ""
            } catch {
                note = "连接失败：\(error.localizedDescription)"
            }
        }
    }

    private func saveAI() {
        guard let engine = store.engine else { return }
        do {
            let current = try engine.aiSettings()
            try engine.setAiSettings(settings: AiSettingsView(
                provider: ProviderView(baseUrl: llmURL, model: llmModel),
                targetLanguage: target,
                glossary: current.glossary,
                syncTranslations: current.syncTranslations,
                syncSummaries: current.syncSummaries,
                dailyAutoLimit: current.dailyAutoLimit,
                rules: current.rules
            ))
            if !llmKey.isEmpty { try engine.setAiKey(key: llmKey) }
            llmKey = ""
            note = "已保存"
            store.reload()
        } catch {
            note = "保存失败：\(error.localizedDescription)"
        }
    }
}

extension DemoStore {
    private static var resetMarker: URL { dataDirectory.deletingLastPathComponent().appendingPathComponent("Eddy-UXDemo.reset") }

    static func requestReset() {
        FileManager.default.createFile(atPath: resetMarker.path, contents: Data())
    }

    /// Clears the demo data before the engine opens it.
    static func applyPendingReset() {
        guard FileManager.default.fileExists(atPath: resetMarker.path) else { return }
        try? FileManager.default.removeItem(at: dataDirectory)
        try? FileManager.default.removeItem(at: resetMarker)
    }
}
