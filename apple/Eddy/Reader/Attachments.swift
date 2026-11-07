import AppKit
import EddyKit
import SwiftMath
import SwiftUI

/// Formulas rendered by SwiftMath at 2x for crisp Retina output.
enum MathRenderer {
    private static var cache: [String: (NSImage, CGFloat)] = [:]

    static func attachment(_ tex: String, size: CGFloat, display: Bool) -> NSTextAttachment {
        let attachment = NSTextAttachment()
        let key = "\(display)|\(size)|\(tex)"
        let rendered: (NSImage, CGFloat)? = cache[key] ?? {
            var image = MathImage(latex: tex, fontSize: size * 2, textColor: .labelColor, labelMode: display ? .display : .text)
            let (_, img, info) = image.asImage()
            guard let img else { return nil }
            // Rendered at 2x; sized once here so cached images are reused as is.
            img.size = NSSize(width: img.size.width / 2, height: img.size.height / 2)
            let value = (img, (info?.descent ?? 0) / 2)
            cache[key] = value
            return value
        }()
        if let (image, descent) = rendered {
            attachment.image = image
            attachment.bounds = CGRect(x: 0, y: -descent, width: image.size.width, height: image.size.height)
        } else {
            // Unparseable TeX: show the source.
            let text = NSAttributedString(string: tex, attributes: [.font: NSFont.monospacedSystemFont(ofSize: size * 0.85, weight: .regular), .foregroundColor: NSColor.secondaryLabelColor])
            let image = NSImage(size: text.size(), flipped: false) { rect in text.draw(in: rect); return true }
            attachment.image = image
            attachment.bounds = CGRect(origin: .zero, size: image.size)
        }
        return attachment
    }
}

/// An image that loads asynchronously and scales to the column width.
final class ImageAttachment: NSTextAttachment {
    let url: String
    let alt: String
    private(set) var naturalSize: CGSize

    init(url: String, alt: String, width: CGFloat?, height: CGFloat?) {
        self.url = url
        self.alt = alt
        if let width, let height, width > 0, height > 0 {
            naturalSize = CGSize(width: width, height: height)
        } else {
            naturalSize = CGSize(width: 640, height: 360)
        }
        super.init(data: nil, ofType: nil)
        image = Self.placeholder
    }

    required init?(coder: NSCoder) { fatalError() }

    private static let placeholder: NSImage = {
        let image = NSImage(size: NSSize(width: 16, height: 9), flipped: false) { rect in
            NSColor.quaternaryLabelColor.withAlphaComponent(0.25).setFill()
            rect.fill()
            return true
        }
        return image
    }()

    func loaded(_ image: NSImage) {
        self.image = image
        if image.size.width > 0 { naturalSize = image.size }
    }

    override func attachmentBounds(
        for attributes: [NSAttributedString.Key: Any],
        location: any NSTextLocation,
        textContainer: NSTextContainer?,
        proposedLineFragment: CGRect,
        position: CGPoint
    ) -> CGRect {
        let maxWidth = max(40, proposedLineFragment.width - 4)
        let width = min(naturalSize.width, maxWidth)
        let height = min(naturalSize.height * width / max(naturalSize.width, 1), 900)
        let finalWidth = naturalSize.height * width / max(naturalSize.width, 1) > 900 ? naturalSize.width * 900 / max(naturalSize.height, 1) : width
        return CGRect(x: 0, y: 0, width: finalWidth, height: height)
    }
}

/// Tables as a hosted SwiftUI grid inside the text.
final class TableAttachment: NSTextAttachment {
    let rows: [TableRowView]
    let style: ReaderStyle
    private var cachedWidth: CGFloat = 0
    private var cachedHeight: CGFloat = 0

    init(rows: [TableRowView], style: ReaderStyle) {
        self.rows = rows
        self.style = style
        super.init(data: nil, ofType: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    func makeView() -> NSView {
        NSHostingView(rootView: TableGrid(rows: rows, fontSize: style.fontSize * 0.85))
    }

    override func viewProvider(for parentView: NSView?, location: any NSTextLocation, textContainer: NSTextContainer?) -> NSTextAttachmentViewProvider? {
        let provider = HostedViewProvider(textAttachment: self, parentView: parentView, textLayoutManager: textContainer?.textLayoutManager, location: location)
        provider.tracksTextAttachmentViewBounds = true
        return provider
    }

    func height(for width: CGFloat) -> CGFloat {
        if abs(width - cachedWidth) > 1 {
            let view = makeView()
            view.frame.size.width = width
            cachedHeight = min(view.fittingSize.height, 2000)
            cachedWidth = width
        }
        return cachedHeight
    }

    override func attachmentBounds(
        for attributes: [NSAttributedString.Key: Any],
        location: any NSTextLocation,
        textContainer: NSTextContainer?,
        proposedLineFragment: CGRect,
        position: CGPoint
    ) -> CGRect {
        let width = max(100, proposedLineFragment.width - 4)
        return CGRect(x: 0, y: 0, width: width, height: height(for: width))
    }
}

final class HostedViewProvider: NSTextAttachmentViewProvider {
    override func loadView() {
        view = (textAttachment as? TableAttachment)?.makeView()
    }
}

private struct TableGrid: View {
    let rows: [TableRowView]
    let fontSize: CGFloat

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                    GridRow {
                        ForEach(Array(row.cells.enumerated()), id: \.offset) { _, cell in
                            Text(cell.runs.map { $0.math ? "$\($0.text)$" : $0.text }.joined())
                                .font(.system(size: fontSize, weight: cell.header ? .semibold : .regular))
                                .gridCellColumns(Int(cell.colspan))
                                .frame(minWidth: 50, alignment: .leading)
                                .textSelection(.enabled)
                        }
                    }
                    Divider()
                }
            }
            .padding(10)
        }
        .background(RoundedRectangle(cornerRadius: 8).stroke(Color.secondary.opacity(0.25)))
    }
}

/// Draws code block backgrounds and quote bars behind TextKit 2 paragraphs.
final class DecoratedFragment: NSTextLayoutFragment {
    enum Kind { case code, quote(Int) }
    var kind: Kind = .code

    override var renderingSurfaceBounds: CGRect {
        super.renderingSurfaceBounds.union(layoutFragmentFrame.offsetBy(dx: -layoutFragmentFrame.minX, dy: -layoutFragmentFrame.minY).insetBy(dx: -8, dy: -6))
    }

    override func draw(at point: CGPoint, in context: CGContext) {
        let frame = CGRect(origin: point, size: layoutFragmentFrame.size)
        switch kind {
        case .code:
            let rect = CGRect(x: frame.minX, y: frame.minY - 2, width: frame.width, height: frame.height - paragraphSpacingBelow() + 4)
            context.saveGState()
            context.setFillColor(NSColor.quaternaryLabelColor.withAlphaComponent(0.18).cgColor)
            context.addPath(CGPath(roundedRect: rect, cornerWidth: 8, cornerHeight: 8, transform: nil))
            context.fillPath()
            context.restoreGState()
        case let .quote(depth):
            context.saveGState()
            context.setFillColor(NSColor.controlAccentColor.withAlphaComponent(0.55).cgColor)
            for level in 0..<depth {
                context.fill(CGRect(x: frame.minX + CGFloat(level) * 20 + 2, y: frame.minY, width: 3, height: frame.height - paragraphSpacingBelow()))
            }
            context.restoreGState()
        }
        super.draw(at: point, in: context)
    }

    private func paragraphSpacingBelow() -> CGFloat {
        guard let paragraph = textElement as? NSTextParagraph, paragraph.attributedString.length > 0,
              let style = paragraph.attributedString.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSParagraphStyle
        else { return 0 }
        return style.paragraphSpacing
    }
}
