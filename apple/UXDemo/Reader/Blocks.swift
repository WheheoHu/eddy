import AppKit
import EddyKit
import SwiftMath
import SwiftUI

enum ReadingMode: String, CaseIterable, Identifiable {
    case original = "原文"
    case bilingual = "双语"
    case translation = "译文"
    var id: String { rawValue }
}

/// Reader typography shared by the directions (each may tune it).
struct Typography: Equatable {
    var size: CGFloat = 17
    var serif = false
    var width: CGFloat = 700
    var lineSpacing: CGFloat = 6

    var bodyFont: Font { .system(size: size, design: serif ? .serif : .default) }
    func heading(_ level: UInt8) -> Font {
        let scale: CGFloat = [1.7, 1.45, 1.25, 1.1, 1.0, 1.0][Int(min(max(level, 1), 6)) - 1]
        return .system(size: size * scale, weight: .bold, design: serif ? .serif : .default)
    }
}

// MARK: - Inline text

struct RunText {
    static func attributed(_ runs: [RunView], typography: Typography, color: Color = .primary) -> Text {
        var out = Text("")
        for run in runs {
            if run.math {
                out = out + InlineMath.text(run.text, size: typography.size)
                continue
            }
            var a = AttributedString(run.text)
            var font = typography.bodyFont
            if run.style & 4 != 0 { font = .system(size: typography.size * 0.9, design: .monospaced) }
            if run.style & 128 != 0 { font = .system(size: typography.size * 0.85, design: typography.serif ? .serif : .default) }
            a.font = font
            if run.style & 1 != 0 { a.inlinePresentationIntent = (a.inlinePresentationIntent ?? []).union(.stronglyEmphasized) }
            if run.style & 2 != 0 { a.inlinePresentationIntent = (a.inlinePresentationIntent ?? []).union(.emphasized) }
            if run.style & 8 != 0 { a.strikethroughStyle = .single }
            if run.style & 16 != 0 { a.underlineStyle = .single }
            if run.style & 32 != 0 { a.baselineOffset = typography.size * 0.35; a.font = .system(size: typography.size * 0.7) }
            if run.style & 64 != 0 { a.baselineOffset = -typography.size * 0.2; a.font = .system(size: typography.size * 0.7) }
            if run.style & 256 != 0 { a.backgroundColor = .yellow.opacity(0.35) }
            if run.style & 4 != 0 { a.backgroundColor = Color.secondary.opacity(0.12) }
            if let link = run.link, let url = URL(string: link) {
                a.link = url
                a.foregroundColor = .accentColor
            } else {
                a.foregroundColor = color
            }
            out = out + Text(a)
        }
        return out
    }
}

/// Inline formulas rendered by SwiftMath and placed on the text baseline.
enum InlineMath {
    private static var cache: [String: (NSImage, CGFloat)] = [:]

    static func text(_ tex: String, size: CGFloat) -> Text {
        let key = "\(size)|\(tex)"
        let rendered: (NSImage, CGFloat)? = cache[key] ?? {
            var image = MathImage(latex: tex, fontSize: size, textColor: .labelColor, labelMode: .text)
            let (_, img, info) = image.asImage()
            guard let img else { return nil }
            let value = (img, info?.descent ?? 0)
            cache[key] = value
            return value
        }()
        guard let (image, descent) = rendered else {
            return Text(tex).font(.system(size: size * 0.9, design: .monospaced))
        }
        return Text(Image(nsImage: image)).baselineOffset(-descent)
    }
}

/// Display equations as a native SwiftMath label.
struct MathBlock: NSViewRepresentable {
    let tex: String
    let size: CGFloat

    func makeNSView(context: Context) -> MTMathUILabel {
        let label = MTMathUILabel()
        label.labelMode = .display
        label.textAlignment = .center
        label.setContentHuggingPriority(.required, for: .vertical)
        return label
    }

    func updateNSView(_ label: MTMathUILabel, context: Context) {
        label.latex = tex
        label.fontSize = size * 1.15
        label.textColor = .labelColor
        label.invalidateIntrinsicContentSize()
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: MTMathUILabel, context: Context) -> CGSize? {
        let size = nsView.intrinsicContentSize
        return CGSize(width: size.width, height: max(size.height, 20))
    }
}

// MARK: - Code

enum CodeColors {
    static func color(_ c: TokenClassView) -> Color {
        switch c {
        case .keyword: return Color(nsColor: .systemPink)
        case .string: return Color(nsColor: .systemRed)
        case .comment: return .secondary
        case .number, .constant: return Color(nsColor: .systemPurple)
        case .function: return Color(nsColor: .systemTeal)
        case .type: return Color(nsColor: .systemBlue)
        case .variable: return Color(nsColor: .systemOrange)
        case .tag: return Color(nsColor: .systemBlue)
        case .attribute: return Color(nsColor: .systemBrown)
        case .operator, .punctuation: return .primary
        }
    }

    static func highlighted(_ text: String, _ spans: [HighlightView], size: CGFloat) -> AttributedString {
        let ns = NSMutableAttributedString(string: text, attributes: [.font: NSFont.monospacedSystemFont(ofSize: size, weight: .regular), .foregroundColor: NSColor.labelColor])
        let length = (text as NSString).length
        for span in spans where Int(span.end) <= length && span.start < span.end {
            let range = NSRange(location: Int(span.start), length: Int(span.end - span.start))
            ns.addAttribute(.foregroundColor, value: NSColor(color(span.class)), range: range)
            if span.class == .comment {
                ns.addAttribute(.font, value: NSFontManager.shared.convert(NSFont.monospacedSystemFont(ofSize: size, weight: .regular), toHaveTrait: .italicFontMask), range: range)
            }
        }
        return (try? AttributedString(ns, including: \.appKit)) ?? AttributedString(text)
    }
}

struct CodeBlock: View {
    let language: String?
    let text: String
    let highlights: [HighlightView]
    let size: CGFloat

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let language {
                Text(language.uppercased()).font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
            }
            ScrollView(.horizontal, showsIndicators: false) {
                Text(CodeColors.highlighted(text, highlights, size: size * 0.8))
                    .textSelection(.enabled)
                    .fixedSize(horizontal: true, vertical: false)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color.secondary.opacity(0.08)))
    }
}

// MARK: - Images

struct EngineImage: View {
    @Environment(DemoStore.self) private var store
    let url: String
    var contentMode: ContentMode = .fit
    @State private var image: NSImage?
    @State private var failed = false

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image).resizable().aspectRatio(contentMode: contentMode)
            } else if failed {
                Rectangle().fill(Color.secondary.opacity(0.08)).overlay(Image(systemName: "photo").foregroundStyle(.tertiary))
            } else {
                Rectangle().fill(Color.secondary.opacity(0.06)).overlay(ProgressView().controlSize(.small))
            }
        }
        .task(id: url) {
            image = await store.image(url)
            failed = image == nil
        }
    }
}

// MARK: - Blocks

struct BlockContent: View {
    let block: EddyKit.BlockView
    let typography: Typography
    let translation: String?
    let mode: ReadingMode

    var body: some View {
        content
            .padding(.leading, CGFloat(block.quoteDepth) * 16)
            .overlay(alignment: .leading) {
                if block.quoteDepth > 0 {
                    Rectangle().fill(Color.accentColor.opacity(0.5)).frame(width: 3)
                }
            }
    }

    @ViewBuilder
    private var content: some View {
        switch block.kind {
        case let .heading(level, runs):
            textBlock(runs, font: typography.heading(level))
                .padding(.top, level <= 2 ? 18 : 10)
        case let .paragraph(runs):
            textBlock(runs, font: typography.bodyFont)
        case let .listItem(ordered, depth, index, marker, runs):
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(marker ? (ordered ? "\(index)." : "•") : " ")
                    .font(typography.bodyFont)
                    .foregroundStyle(.secondary)
                    .frame(width: 22, alignment: .trailing)
                textBlock(runs, font: typography.bodyFont)
            }
            .padding(.leading, CGFloat(depth) * 22)
        case let .code(language, text, highlights):
            CodeBlock(language: language, text: text, highlights: highlights, size: typography.size)
        case let .math(tex):
            ScrollView(.horizontal, showsIndicators: false) {
                MathBlock(tex: tex, size: typography.size)
                    .frame(maxWidth: .infinity)
            }
            .padding(.vertical, 6)
        case let .image(src, alt, _, _, caption):
            VStack(spacing: 6) {
                EngineImage(url: src)
                    .frame(maxWidth: .infinity, maxHeight: 520)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                    .accessibilityLabel(alt)
                if !caption.isEmpty {
                    RunText.attributed(caption, typography: Typography(size: typography.size * 0.8, serif: typography.serif), color: .secondary)
                        .multilineTextAlignment(.center)
                }
                translationText(font: .system(size: typography.size * 0.8))
            }
        case let .table(rows):
            TableBlock(rows: rows, typography: typography)
        case let .media(kind, src, _, title):
            Link(destination: URL(string: src) ?? URL(string: "about:blank")!) {
                Label(title ?? (kind == .embed ? "嵌入内容" : kind == .video ? "视频" : "音频"), systemImage: kind == .audio ? "waveform" : "play.rectangle")
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 8).fill(Color.secondary.opacity(0.08)))
            }
            .buttonStyle(.plain)
        case .rule:
            Divider().padding(.vertical, 8)
        }
    }

    @ViewBuilder
    private func textBlock(_ runs: [RunView], font: Font) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            if mode != .translation || translation == nil {
                RunText.attributed(runs, typography: typographyFor(font))
                    .lineSpacing(typography.lineSpacing)
                    .textSelection(.enabled)
            }
            translationText(font: font)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private func translationText(font: Font) -> some View {
        if mode != .original, let translation {
            Text(translation)
                .font(font)
                .foregroundStyle(mode == .bilingual ? Color.secondary : Color.primary)
                .lineSpacing(typography.lineSpacing)
                .textSelection(.enabled)
        }
    }

    private func typographyFor(_ font: Font) -> Typography {
        guard case let .heading(level, _) = block.kind else { return typography }
        var t = typography
        t.size = typography.size * [1.7, 1.45, 1.25, 1.1, 1.0, 1.0][Int(min(max(level, 1), 6)) - 1]
        return t
    }
}


struct TableBlock: View {
    let rows: [TableRowView]
    let typography: Typography

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 8) {
                ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                    GridRow {
                        ForEach(Array(row.cells.enumerated()), id: \.offset) { _, cell in
                            RunText.attributed(cell.runs, typography: Typography(size: typography.size * 0.85, serif: typography.serif))
                                .fontWeight(cell.header ? .semibold : .regular)
                                .gridCellColumns(Int(cell.colspan))
                                .frame(minWidth: 60, alignment: .leading)
                        }
                    }
                    Divider()
                }
            }
            .padding(12)
        }
        .background(RoundedRectangle(cornerRadius: 8).stroke(Color.secondary.opacity(0.2)))
    }
}
