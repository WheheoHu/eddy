import EddyKit
import SwiftUI

@main
struct EddyApp: App {
    @State private var pong = "…"

    var body: some Scene {
        WindowGroup("Eddy") {
            VStack(spacing: 12) {
                Text("Eddy core \(EddyCore.version)")
                Text(pong).foregroundStyle(.secondary)
            }
            .frame(minWidth: 480, minHeight: 320)
            .task { pong = await ping(delayMs: 10) }
        }
    }
}
