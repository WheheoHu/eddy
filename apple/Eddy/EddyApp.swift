import EddyKit
import SwiftUI

@main
struct EddyApp: App {
    @State private var model = DebugModel()

    var body: some Scene {
        WindowGroup("Eddy") {
            DebugView(model: model)
                .frame(minWidth: 900, minHeight: 560)
        }
    }
}
