import AppKit
import EddyKit
import SwiftMath
import SwiftUI

/// Formulas rendered by SwiftMath at 2x for crisp Retina output.
enum MathRenderer {
    private static var cache: [String: (NSImage, CGFloat)] = [:]

    /// Drawn in the current drawing appearance, which the key includes:
    /// the formula's color is baked into the image.
    static func attachment(_ tex: String, size: CGFloat, display: Bool) -> MathAttachment {
        let key = "\(NSAppearance.currentDrawing().name.rawValue)|\(display)|\(size)|\(tex)"
        let rendered: (NSImage, CGFloat)? = cache[key] ?? {
            var image = MathImage(latex: normalized(tex), fontSize: size * 2, textColor: .labelColor, labelMode: display ? .display : .text)
            let (_, img, info) = image.asImage()
            guard let img else { return nil }
            // Rendered at 2x; sized once here so cached images are reused as is.
            img.size = NSSize(width: img.size.width / 2, height: img.size.height / 2)
            let value = (img, (info?.descent ?? 0) / 2)
            cache[key] = value
            return value
        }()
        if let (image, descent) = rendered {
            return MathAttachment(image: image, descent: descent, display: display)
        }
        // Unparseable TeX: show the source.
        let text = NSAttributedString(string: tex, attributes: [.font: NSFont.monospacedSystemFont(ofSize: size * 0.85, weight: .regular), .foregroundColor: NSColor.secondaryLabelColor])
        let image = NSImage(size: text.size(), flipped: false) { rect in text.draw(in: rect); return true }
        return MathAttachment(image: image, descent: 0, display: display)
    }

    /// Rewrites what SwiftMath cannot parse (equation tags, manual delimiter
    /// sizes, a few AMS spellings) into what it can, so the formula renders
    /// instead of falling back to its source.
    static func normalized(_ tex: String) -> String {
        var s = tex
        var tag: String?
        if let range = s.range(of: #"\\tag(\*?)\{([^{}]*)\}"#, options: .regularExpression) {
            let match = s[range]
            let number = match.drop { $0 != "{" }.dropFirst().dropLast()
            tag = match.hasPrefix(#"\tag*"#) ? String(number) : "(\(number))"
            s.removeSubrange(range)
        }
        for (pattern, replacement) in replacements {
            s = s.replacingOccurrences(of: pattern, with: replacement, options: .regularExpression)
        }
        if let tag { s += #"\qquad \text{"# + tag + "}" }
        return s
    }

    private static let replacements: [(String, String)] = [
        (#"\\[Bb]igg?[lmr]?(?![a-zA-Z])"#, ""),
        // Font size switches mean nothing to a one-size renderer.
        (#"\\(tiny|scriptsize|footnotesize|small|normalsize|large|Large|LARGE|huge|Huge)(?![a-zA-Z])\s?"#, ""),
        (#"\\operatorname\*?(?![a-zA-Z])"#, #"\\mathrm"#),
        (#"\\[dt]frac(?![a-zA-Z])"#, #"\\frac"#),
        (#"\\widehat(?![a-zA-Z])"#, #"\\hat"#),
        (#"\\widetilde(?![a-zA-Z])"#, #"\\tilde"#),
        (#"\\[lr]Vert(?![a-zA-Z])"#, #"\\Vert"#),
        (#"\\[lr]vert(?![a-zA-Z])"#, #"\\vert"#),
    ]
}

/// A rendered formula. It never takes more than its line: TextKit 2 makes
/// room for wider content by moving the whole viewport sideways, which
/// shifts the article out of the column with no way to scroll it back. A
/// display formula that is too wide scrolls sideways in its own line; an
/// inline one shrinks to fit.
final class MathAttachment: NSTextAttachment {
    let naturalSize: CGSize
    let descent: CGFloat
    let display: Bool

    init(image: NSImage, descent: CGFloat, display: Bool) {
        naturalSize = image.size
        self.descent = descent
        self.display = display
        super.init(data: nil, ofType: nil)
        self.image = image
    }

    required init?(coder: NSCoder) { fatalError() }

    /// Room under a scrolling formula for its scroller.
    static var scrollerHeight: CGFloat {
        NSScroller.preferredScrollerStyle == .legacy ? NSScroller.scrollerWidth(for: .regular, scrollerStyle: .legacy) : 10
    }

    func scrolls(in width: CGFloat) -> Bool {
        display && naturalSize.width > width + 0.5
    }

    override func attachmentBounds(
        for attributes: [NSAttributedString.Key: Any],
        location: any NSTextLocation,
        textContainer: NSTextContainer?,
        proposedLineFragment: CGRect,
        position: CGPoint
    ) -> CGRect {
        let width = proposedLineFragment.width
        if naturalSize.width <= width + 0.5 {
            return CGRect(x: 0, y: -descent, width: naturalSize.width, height: naturalSize.height)
        }
        if display {
            let scroller = Self.scrollerHeight
            return CGRect(x: 0, y: -descent - scroller, width: width, height: naturalSize.height + scroller)
        }
        let scale = width / naturalSize.width
        return CGRect(x: 0, y: -descent * scale, width: width, height: naturalSize.height * scale)
    }

    override func viewProvider(for parentView: NSView?, location: any NSTextLocation, textContainer: NSTextContainer?) -> NSTextAttachmentViewProvider? {
        guard let textContainer, scrolls(in: textContainer.size.width - 2 * textContainer.lineFragmentPadding) else { return nil }
        return ScrollingMathProvider(textAttachment: self, parentView: parentView, textLayoutManager: textContainer.textLayoutManager, location: location)
    }
}

/// A wide display formula in a horizontal scroll view the size of its line.
final class ScrollingMathProvider: NSTextAttachmentViewProvider {
    override func loadView() {
        guard let attachment = textAttachment as? MathAttachment, let image = attachment.image else { return }
        // Flipped, so the formula sits at the top and the scroller below it.
        let content = FlippedView(frame: CGRect(origin: .zero, size: attachment.naturalSize))
        let imageView = NSImageView(image: image)
        imageView.imageScaling = .scaleNone
        imageView.frame = content.bounds
        content.addSubview(imageView)
        let scroll = SidewaysScrollView()
        scroll.drawsBackground = false
        scroll.hasHorizontalScroller = true
        scroll.hasVerticalScroller = false
        scroll.verticalScrollElasticity = .none
        scroll.documentView = content
        view = scroll
    }

    private final class FlippedView: NSView {
        override var isFlipped: Bool { true }
    }

    /// Scrolls sideways only; vertical scrolling goes on to the article, so
    /// reading past a formula does not stall under the pointer. A trackpad
    /// gesture keeps the direction it started with.
    private final class SidewaysScrollView: NSScrollView {
        private var vertical = false

        override func scrollWheel(with event: NSEvent) {
            if event.phase == .began || (event.phase.isEmpty && event.momentumPhase.isEmpty) {
                vertical = abs(event.scrollingDeltaY) > abs(event.scrollingDeltaX)
            }
            if vertical {
                nextResponder?.scrollWheel(with: event)
            } else {
                super.scrollWheel(with: event)
            }
        }
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

    private(set) var isLoaded = false

    func loaded(_ image: NSImage) {
        self.image = image
        if image.size.width > 0 { naturalSize = image.size }
        isLoaded = true
    }

    /// The size the image takes in a line `lineWidth` wide: scaled down to
    /// the column, and no taller than a screen.
    func displaySize(forLineWidth lineWidth: CGFloat) -> CGSize {
        let maxWidth = max(40, lineWidth - 4)
        let width = min(naturalSize.width, maxWidth)
        let height = min(naturalSize.height * width / max(naturalSize.width, 1), 900)
        let finalWidth = naturalSize.height * width / max(naturalSize.width, 1) > 900 ? naturalSize.width * 900 / max(naturalSize.height, 1) : width
        return CGSize(width: finalWidth, height: height)
    }

    func displayHeight(forLineWidth lineWidth: CGFloat) -> CGFloat {
        displaySize(forLineWidth: lineWidth).height
    }

    override func attachmentBounds(
        for attributes: [NSAttributedString.Key: Any],
        location: any NSTextLocation,
        textContainer: NSTextContainer?,
        proposedLineFragment: CGRect,
        position: CGPoint
    ) -> CGRect {
        CGRect(origin: .zero, size: displaySize(forLineWidth: proposedLineFragment.width))
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

/// What a code block is, for the label in its corner and the menu behind
/// it. The value of the `.eddyCode` attribute on the block's paragraph.
final class CodeBlockMark: NSObject {
    let blockID: String
    /// As declared, guessed or picked; nil for plain text.
    let language: String?
    /// Where the block starts, inside a quote.
    let indent: CGFloat
    /// How far the code is scrolled sideways; lines never wrap.
    var scrollX: CGFloat = 0

    init(blockID: String, language: String?, indent: CGFloat = 0) {
        self.blockID = blockID
        self.language = language
        self.indent = indent
    }

    /// Height of the strip above the code holding the label.
    static let stripHeight: CGFloat = 22
    /// Space between the block's edge and its code.
    static let inset: CGFloat = 14
    /// Space between the last line of code and the block's bottom edge.
    static let bottomInset: CGFloat = 10

    /// How the label shows a language token: "cpp" as "C++".
    static func displayName(_ language: String?) -> String {
        guard let language = language?.lowercased(), !language.isEmpty else { return "Text" }
        let names: [String: String] = [
            "plaintext": "Text", "plain": "Text", "text": "Text", "txt": "Text", "none": "Text", "nohighlight": "Text",
            "c": "C", "cpp": "C++", "c++": "C++", "cc": "C++", "cxx": "C++", "h": "C", "hpp": "C++",
            "csharp": "C#", "cs": "C#", "c#": "C#", "objc": "Objective-C", "m": "Objective-C",
            "js": "JavaScript", "javascript": "JavaScript", "jsx": "JSX", "mjs": "JavaScript",
            "ts": "TypeScript", "typescript": "TypeScript", "tsx": "TSX",
            "py": "Python", "python": "Python", "python3": "Python", "rb": "Ruby", "ruby": "Ruby",
            "rs": "Rust", "rust": "Rust", "go": "Go", "golang": "Go", "java": "Java", "kt": "Kotlin", "kotlin": "Kotlin",
            "swift": "Swift", "php": "PHP", "lua": "Lua", "scala": "Scala", "dart": "Dart", "zig": "Zig",
            "sh": "Shell", "shell": "Shell", "bash": "Bash", "zsh": "Zsh", "console": "Console", "fish": "Fish", "powershell": "PowerShell",
            "sql": "SQL", "html": "HTML", "xml": "XML", "svg": "SVG", "css": "CSS", "scss": "SCSS", "json": "JSON",
            "yaml": "YAML", "yml": "YAML", "toml": "TOML", "ini": "INI", "md": "Markdown", "markdown": "Markdown",
            "tex": "TeX", "latex": "LaTeX", "diff": "Diff", "patch": "Diff", "makefile": "Makefile", "make": "Makefile",
            "dockerfile": "Dockerfile", "glsl": "GLSL", "hlsl": "HLSL", "wgsl": "WGSL", "metal": "Metal", "cuda": "CUDA",
            "hs": "Haskell", "haskell": "Haskell", "ml": "OCaml", "ocaml": "OCaml", "ex": "Elixir", "elixir": "Elixir",
            "clj": "Clojure", "clojure": "Clojure", "erl": "Erlang", "erlang": "Erlang", "r": "R", "matlab": "MATLAB",
            "perl": "Perl", "pl": "Perl",
        ]
        if let name = names[language] { return name }
        return language.prefix(1).uppercased() + language.dropFirst()
    }

    /// Whether the token means no highlighting.
    static func isPlain(_ language: String?) -> Bool {
        guard let language = language?.lowercased() else { return true }
        return ["plaintext", "plain", "text", "txt", "none", "nohighlight"].contains(language)
    }
}

/// Draws code block backgrounds and quote bars behind TextKit 2 paragraphs.
/// A code block gets its language in the corner of a strip above the code;
/// `labelRect` (in the fragment's own coordinates) is where that is, for
/// clicks on it.
final class DecoratedFragment: NSTextLayoutFragment {
    enum Kind { case code(CodeBlockMark?), quote(Int) }
    var kind: Kind = .code(nil)
    private(set) var labelRect: CGRect = .zero
    /// How much wider the code is than the room it has; it scrolls by that.
    private(set) var codeOverflow: CGFloat = 0

    /// The code block's box, in the fragment's own coordinates: the whole
    /// column, whatever the length of the code.
    private var codeBox: CGRect {
        let width = textLayoutManager?.textContainer?.size.width ?? layoutFragmentFrame.width
        let indent = codeMark?.indent ?? 0
        return CGRect(
            x: indent - layoutFragmentFrame.minX, y: -2,
            width: width - indent, height: layoutFragmentFrame.height - paragraphSpacingBelow() + 4 + CodeBlockMark.bottomInset
        )
    }

    override var renderingSurfaceBounds: CGRect {
        let own = layoutFragmentFrame.offsetBy(dx: -layoutFragmentFrame.minX, dy: -layoutFragmentFrame.minY).insetBy(dx: -8, dy: -6)
        if case .code = kind {
            return super.renderingSurfaceBounds.union(own).union(codeBox.insetBy(dx: -2, dy: -6))
        }
        return super.renderingSurfaceBounds.union(own)
    }

    override func draw(at point: CGPoint, in context: CGContext) {
        let frame = CGRect(origin: point, size: layoutFragmentFrame.size)
        switch kind {
        case let .code(mark):
            let rect = codeBox.offsetBy(dx: point.x, dy: point.y)
            context.saveGState()
            context.setFillColor(NSColor.quaternaryLabelColor.withAlphaComponent(0.18).cgColor)
            context.addPath(CGPath(roundedRect: rect, cornerWidth: 8, cornerHeight: 8, transform: nil))
            context.fillPath()
            context.restoreGState()
            drawLabel(mark, in: rect, at: point, context: context)
            // The code at its full length, clipped to the box and shifted
            // by its scroll. TextKit laid the lines out clipped (so they
            // never wrap and the block has the right height); the glyphs
            // past the edge come from drawing each line again whole.
            let inset = CodeBlockMark.inset
            let room = rect.width - 2 * inset
            let lines = codeLines
            codeOverflow = max(0, (lines.map(\.width).max() ?? 0) - room)
            let scroll = min(mark?.scrollX ?? 0, codeOverflow)
            mark?.scrollX = scroll
            context.saveGState()
            context.clip(to: CGRect(x: rect.minX + inset - 4, y: rect.minY, width: room + 8, height: rect.height))
            context.textMatrix = .identity
            for (fragment, line) in zip(textLineFragments, lines) {
                let bounds = fragment.typographicBounds
                let baseline = CGPoint(
                    x: point.x + bounds.minX + fragment.glyphOrigin.x - scroll,
                    y: point.y + bounds.minY + fragment.glyphOrigin.y
                )
                context.saveGState()
                context.translateBy(x: baseline.x, y: baseline.y)
                context.scaleBy(x: 1, y: -1)
                // The text position isn't part of the graphics state: the
                // previous line left it at its own end.
                context.textPosition = .zero
                CTLineDraw(line.line, context)
                context.restoreGState()
            }
            context.restoreGState()
            return
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

    /// The language, top right, with a mark that it opens a menu.
    private func drawLabel(_ mark: CodeBlockMark?, in rect: CGRect, at point: CGPoint, context: CGContext) {
        let text = CodeBlockMark.displayName(mark?.language) + " ▾"
        let label = NSAttributedString(string: text, attributes: [
            .font: NSFont.systemFont(ofSize: 11, weight: .medium),
            .foregroundColor: NSColor.secondaryLabelColor,
        ])
        let size = label.size()
        let origin = CGPoint(x: rect.maxX - 12 - size.width, y: rect.minY + (CodeBlockMark.stripHeight - size.height) / 2 + 2)
        // Generous, for clicking: the whole corner of the strip.
        labelRect = CGRect(x: origin.x - 10, y: rect.minY, width: size.width + 22, height: CodeBlockMark.stripHeight + 2)
            .offsetBy(dx: -point.x, dy: -point.y)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
        label.draw(at: origin)
        NSGraphicsContext.restoreGraphicsState()
    }

    private var builtCodeLines: [(line: CTLine, width: CGFloat)]?

    /// Each line of the code set whole, with its width, from the text
    /// TextKit gave each line fragment.
    private var codeLines: [(line: CTLine, width: CGFloat)] {
        if let builtCodeLines { return builtCodeLines }
        guard let paragraph = textElement as? NSTextParagraph else { return [] }
        let text = paragraph.attributedString
        let lines = textLineFragments.map { fragment in
            let range = NSIntersectionRange(fragment.characterRange, NSRange(location: 0, length: text.length))
            let piece = text.attributedSubstring(from: range).mutableCopy() as! NSMutableAttributedString
            // Line separators and the final newline take no room.
            while piece.length > 0, let last = piece.string.unicodeScalars.last, last == "\u{2028}" || last == "\n" {
                piece.deleteCharacters(in: NSRange(location: piece.length - 1, length: 1))
            }
            let line = CTLineCreateWithAttributedString(piece)
            return (line: line, width: CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil)))
        }
        builtCodeLines = lines
        return lines
    }

    /// The code block's mark, when this is one.
    var codeMark: CodeBlockMark? {
        if case let .code(mark) = kind { return mark }
        return nil
    }

    /// Scrolls the code sideways by `delta` points; whether anything moved.
    func scrollCode(by delta: CGFloat) -> Bool {
        guard let mark = codeMark, codeOverflow > 0 else { return false }
        let target = min(max(0, mark.scrollX + delta), codeOverflow)
        guard target != mark.scrollX else { return false }
        mark.scrollX = target
        return true
    }

    private func paragraphSpacingBelow() -> CGFloat {
        guard let paragraph = textElement as? NSTextParagraph, paragraph.attributedString.length > 0,
              let style = paragraph.attributedString.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSParagraphStyle
        else { return 0 }
        return style.paragraphSpacing
    }
}
