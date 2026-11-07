import EddyKit
import Observation
import SwiftUI

/// Reading progress published by `ArticleView`. Only the views that draw
/// progress observe it, so scrolling never re-renders the article itself
/// (re-rendering it while scrolling feeds back into the lazy layout).
@Observable
@MainActor
final class ReaderProgress {
    var value = 0.0

    func update(_ new: Double) {
        if abs(new - value) >= 0.005 { value = new }
    }
}

/// Visibility of floating reader controls: hidden while scrolling, shown
/// again when the pointer nears the top or reading pauses.
@Observable
@MainActor
final class ChromeState {
    var visible = true
    private var hideTask: Task<Void, Never>?

    func scrolled() {
        guard visible else { return }
        hideTask?.cancel()
        hideTask = Task {
            try? await Task.sleep(for: .seconds(1.2))
            if !Task.isCancelled { visible = false }
        }
    }

    func reveal() {
        hideTask?.cancel()
        if !visible { visible = true }
    }
}

struct ProgressLine: View {
    let progress: ReaderProgress

    var body: some View {
        GeometryReader { geo in
            Rectangle()
                .fill(Color.accentColor)
                .frame(width: geo.size.width * progress.value, height: 3)
                .animation(.linear(duration: 0.15), value: progress.value)
        }
        .frame(height: 3)
        .allowsHitTesting(false)
    }
}

/// Previous/next navigation through an ordered list (chapters of a
/// publication, stories of an issue).
struct SequenceBar: View {
    let title: String
    let items: [ItemView]
    let current: String
    let open: (String) -> Void

    var body: some View {
        let index = items.firstIndex { $0.id == current } ?? 0
        HStack {
            Button { open(items[index - 1].id) } label: { Label("上一篇", systemImage: "chevron.left") }
                .disabled(index == 0)
            Spacer()
            Menu("\(title) · \(index + 1) / \(items.count)") {
                ForEach(items) { item in
                    Button((item.finished ? "✓ " : item.progress > 0.02 ? "◐ " : "") + item.title) { open(item.id) }
                }
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            Spacer()
            Button { open(items[index + 1].id) } label: { Label("下一篇", systemImage: "chevron.right") }
                .disabled(index + 1 >= items.count)
        }
        .buttonStyle(.borderless)
        .padding(.horizontal, 20)
        .padding(.vertical, 10)
        .background(.bar)
    }
}
