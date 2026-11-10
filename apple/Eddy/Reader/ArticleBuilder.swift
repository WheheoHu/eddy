import AppKit
import EddyKit
import SwiftMath
import SwiftUI

/// Reader typography. Stored in user defaults by the reading settings.
struct ReaderStyle: Equatable {
    var fontSize: CGFloat = 17
    var serif = false
    var lineHeight: CGFloat = 1.45
    var maxWidth: CGFloat = 700

    func font(size: CGFloat? = nil, weight: NSFont.Weight = .regular) -> NSFont {
        let base = NSFont.systemFont(ofSize: size ?? fontSize, weight: weight)
        guard serif, let descriptor = base.fontDescriptor.withDesign(.serif) else { return base }
        return NSFont(descriptor: descriptor, size: size ?? fontSize) ?? base
    }

    func mono(scale: CGFloat = 0.86) -> NSFont {
        .monospacedSystemFont(ofSize: fontSize * scale, weight: .regular)
    }
}

/// How the reader's content is colored: with the rest of the app, or
/// always light or dark. Stored in user defaults as `reader.appearance`.
enum ReaderAppearance: String, CaseIterable, Identifiable {
    case app = "Follow App"
    case light = "Light"
    case dark = "Dark"
    var id: String { rawValue }

    /// The scheme the content uses inside an app showing `app`.
    func scheme(in app: ColorScheme) -> ColorScheme {
        switch self {
        case .app: app
        case .light: .light
        case .dark: .dark
        }
    }

    /// For the reader's AppKit views; nil inherits the app's.
    var nsAppearance: NSAppearance? {
        switch self {
        case .app: nil
        case .light: NSAppearance(named: .aqua)
        case .dark: NSAppearance(named: .darkAqua)
        }
    }
}

extension ColorScheme {
    /// What drawing in this scheme resolves dynamic colors against.
    var drawingAppearance: NSAppearance {
        NSAppearance(named: self == .dark ? .darkAqua : .aqua) ?? NSAppearance.currentDrawing()
    }
}

enum ReadingMode: String, CaseIterable, Identifiable {
    case original = "Original"
    case bilingual = "Bilingual"
    case translation = "Translation"
    var id: String { rawValue }
}

extension NSAttributedString.Key {
    /// Paragraph decoration drawn by `DecoratedFragment`.
    static let eddyCode = NSAttributedString.Key("eddy.code")
    static let eddyQuote = NSAttributedString.Key("eddy.quote")
}

/// The article as one attributed string plus where each source block lives
/// in it (UTF-16 ranges), for reading positions and outline jumps.
struct BuiltArticle {
    let text: NSAttributedString
    let blocks: [(id: String, range: NSRange)]
    /// Translations shown after their block (bilingual), by segment ID.
    var translations: [(id: String, range: NSRange)] = []
    let images: [ImageAttachment]

    /// A place in the text: a block, or the translation shown after one.
    enum Spot: Equatable {
        case block(String)
        case translation(String)
    }

    func spot(at location: Int) -> (spot: Spot, offset: Int)? {
        if let t = translations.first(where: { NSLocationInRange(location, $0.range) }) {
            return (.translation(t.id), location - t.range.location)
        }
        return block(at: location).map { (.block($0.id), $0.offset) }
    }

    /// A translation no longer shown stands in for the end of its block.
    func location(of spot: Spot, offset: Int) -> Int? {
        switch spot {
        case let .block(id):
            return location(of: id, offset: offset)
        case let .translation(id):
            if let t = translations.first(where: { $0.id == id }) {
                return t.range.location + min(offset, t.range.length)
            }
            let blockID = String(id.split(separator: "#").first ?? Substring(id))
            return location(of: blockID, offset: .max)
        }
    }

    func block(at location: Int) -> (id: String, offset: Int)? {
        var lo = 0, hi = blocks.count - 1, found: Int?
        while lo <= hi {
            let mid = (lo + hi) / 2
            if blocks[mid].range.location <= location { found = mid; lo = mid + 1 } else { hi = mid - 1 }
        }
        guard let i = found else { return blocks.first.map { ($0.id, 0) } }
        let b = blocks[i]
        return (b.id, max(0, min(location - b.range.location, b.range.length)))
    }

    func location(of blockID: String, offset: Int) -> Int? {
        guard let b = blocks.first(where: { $0.id == blockID }) else { return nil }
        return b.range.location + min(offset, b.range.length)
    }
}

enum ArticleBuilder {
    static func build(
        doc: DocumentView,
        item: ItemView?,
        translations: [String: String],
        mode: ReadingMode,
        style: ReaderStyle
    ) -> BuiltArticle {
        var b = Builder(style: style, translations: mode == .original ? [:] : translations, mode: mode)
        if let item { b.header(item, translatedTitle: mode == .original ? nil : translations["title"]) }
        for block in doc.blocks { b.block(block) }
        return BuiltArticle(text: b.out, blocks: b.ranges, translations: b.translationRanges, images: b.images)
    }
}

private struct Builder {
    let style: ReaderStyle
    let translations: [String: String]
    let mode: ReadingMode
    let out = NSMutableAttributedString()
    var ranges: [(id: String, range: NSRange)] = []
    var translationRanges: [(id: String, range: NSRange)] = []
    var images: [ImageAttachment] = []

    init(style: ReaderStyle, translations: [String: String], mode: ReadingMode) {
        self.style = style
        self.translations = translations
        self.mode = mode
    }

    // MARK: Paragraph styles

    func paragraph(spacingBefore: CGFloat = 0, spacing: CGFloat? = nil, indent: CGFloat = 0, firstIndent: CGFloat? = nil, align: NSTextAlignment = .natural, lineHeight: CGFloat? = nil) -> NSParagraphStyle {
        let p = NSMutableParagraphStyle()
        p.lineHeightMultiple = lineHeight ?? style.lineHeight
        p.paragraphSpacing = spacing ?? style.fontSize * 0.75
        p.paragraphSpacingBefore = spacingBefore
        p.headIndent = indent
        p.firstLineHeadIndent = firstIndent ?? indent
        p.alignment = align
        if indent > 0 { p.tabStops = [NSTextTab(textAlignment: .left, location: indent)] }
        return p
    }

    func base(_ font: NSFont, _ p: NSParagraphStyle, color: NSColor = .labelColor) -> [NSAttributedString.Key: Any] {
        [.font: font, .paragraphStyle: p, .foregroundColor: color]
    }

    // MARK: Header

    mutating func header(_ item: ItemView, translatedTitle: String?) {
        let small = NSFont.systemFont(ofSize: style.fontSize * 0.72, weight: .semibold)
        out.append(NSAttributedString(string: item.sourceName.uppercased() + "\n", attributes: [
            .font: small, .foregroundColor: NSColor.controlAccentColor, .kern: 0.8,
            .paragraphStyle: paragraph(spacing: 6),
        ]))
        out.append(NSAttributedString(string: item.title + "\n", attributes: base(style.font(size: style.fontSize * 1.9, weight: .bold), paragraph(spacing: 8, lineHeight: 1.1))))
        if let translatedTitle {
            out.append(NSAttributedString(string: translatedTitle + "\n", attributes: base(style.font(size: style.fontSize * 1.3, weight: .semibold), paragraph(spacing: 8, lineHeight: 1.15), color: .secondaryLabelColor)))
        }
        var meta: [String] = []
        if let author = item.author, !author.isEmpty { meta.append(author) }
        meta.append(item.date.formatted(date: .abbreviated, time: .omitted))
        meta.append("\(item.readingMinutes) min read")
        out.append(NSAttributedString(string: meta.joined(separator: " · ") + "\n", attributes: base(.systemFont(ofSize: style.fontSize * 0.8), paragraph(spacing: style.fontSize * 1.6), color: .secondaryLabelColor)))
    }

    // MARK: Blocks

    mutating func block(_ block: EddyKit.BlockView) {
        let quote = Int(block.quoteDepth)
        let quoteIndent = CGFloat(quote) * 20
        switch block.kind {
        case let .heading(level, runs):
            let scale: CGFloat = [1.6, 1.38, 1.2, 1.08, 1.0, 1.0][Int(min(max(level, 1), 6)) - 1]
            let font = style.font(size: style.fontSize * scale, weight: .bold)
            textBlock(block.id, runs, font: font, p: paragraph(spacingBefore: style.fontSize * (level <= 2 ? 1.2 : 0.7), spacing: style.fontSize * 0.45, indent: quoteIndent, lineHeight: 1.2), quote: quote)
        case let .paragraph(runs):
            textBlock(block.id, runs, font: style.font(), p: paragraph(indent: quoteIndent), quote: quote)
        case let .listItem(ordered, depth, index, marker, runs):
            let indent = quoteIndent + CGFloat(depth + 1) * 26
            let p = paragraph(spacing: style.fontSize * 0.35, indent: indent, firstIndent: indent - 22)
            let prefix = marker ? (ordered ? "\(index).\t" : "•\t") : "\t"
            out.append(NSAttributedString(string: prefix, attributes: base(style.font(), p, color: .secondaryLabelColor)))
            textBlock(block.id, runs, font: style.font(), p: p, quote: quote)
        case let .code(language, text, highlights):
            codeBlock(block.id, language: language, text: text, highlights: highlights, indent: quoteIndent)
        case let .math(tex):
            let start = out.length
            let attachment = MathRenderer.attachment(tex, size: style.fontSize * 1.15, display: true)
            let s = NSMutableAttributedString(attachment: attachment)
            s.addAttributes([.paragraphStyle: paragraph(spacingBefore: 4, spacing: style.fontSize * 0.9, align: .center)], range: NSRange(location: 0, length: s.length))
            out.append(s)
            out.append(NSAttributedString(string: "\n", attributes: [.paragraphStyle: paragraph(align: .center)]))
            ranges.append((block.id, NSRange(location: start, length: 1)))
        case let .image(src, alt, width, height, caption):
            let start = out.length
            let attachment = ImageAttachment(url: src, alt: alt, width: width.map { CGFloat($0) }, height: height.map { CGFloat($0) })
            images.append(attachment)
            let s = NSMutableAttributedString(attachment: attachment)
            s.addAttributes([.paragraphStyle: paragraph(spacingBefore: 4, spacing: caption.isEmpty ? style.fontSize : 6, align: .center)], range: NSRange(location: 0, length: s.length))
            out.append(s)
            out.append(NSAttributedString(string: "\n", attributes: [.paragraphStyle: paragraph(align: .center)]))
            ranges.append((block.id, NSRange(location: start, length: 1)))
            if !caption.isEmpty {
                let captionFont = style.font(size: style.fontSize * 0.8)
                out.append(inline(caption, font: captionFont, color: .secondaryLabelColor, p: paragraph(spacing: style.fontSize, align: .center)))
                out.append(NSAttributedString(string: "\n"))
                translation(for: "\(block.id)#caption", font: captionFont, p: paragraph(spacing: style.fontSize, align: .center))
            }
        case let .table(rows):
            let start = out.length
            let attachment = TableAttachment(rows: rows, style: style)
            let s = NSMutableAttributedString(attachment: attachment)
            s.addAttributes([.paragraphStyle: paragraph(spacing: style.fontSize)], range: NSRange(location: 0, length: s.length))
            out.append(s)
            out.append(NSAttributedString(string: "\n"))
            ranges.append((block.id, NSRange(location: start, length: 1)))
        case let .media(kind, src, poster, title):
            let start = out.length
            let label = (kind == .audio ? "♪ " : "▶︎ ") + (title ?? (kind == .embed ? "Embedded content" : kind == .video ? "Video" : "Audio"))
            // Videos and embeds play in the media viewer; audio opens as before.
            let link = kind == .audio ? URL(string: src) : Self.playLink(kind: kind, src: src, poster: poster)
            if kind != .audio, let poster {
                let attachment = ImageAttachment(url: poster, alt: label, width: nil, height: nil)
                images.append(attachment)
                let s = NSMutableAttributedString(attachment: attachment)
                var attrs: [NSAttributedString.Key: Any] = [.paragraphStyle: paragraph(spacingBefore: 4, spacing: 4, align: .center)]
                if let link { attrs[.link] = link }
                s.addAttributes(attrs, range: NSRange(location: 0, length: s.length))
                out.append(s)
                out.append(NSAttributedString(string: "\n", attributes: [.paragraphStyle: paragraph(align: .center)]))
            }
            var attrs = base(style.font(weight: .medium), paragraph(align: kind != .audio && poster != nil ? .center : .natural))
            if let link { attrs[.link] = link }
            out.append(NSAttributedString(string: label + "\n", attributes: attrs))
            ranges.append((block.id, NSRange(location: start, length: out.length - start - 1)))
        case .rule:
            let start = out.length
            out.append(NSAttributedString(string: "⁂\n", attributes: base(style.font(), paragraph(spacingBefore: 6, spacing: style.fontSize, align: .center), color: .tertiaryLabelColor)))
            ranges.append((block.id, NSRange(location: start, length: 1)))
        }
    }

    /// An `eddy-media:` link the reader hands to the media viewer.
    static func playLink(kind: MediaKindView, src: String, poster: String?) -> URL? {
        var components = URLComponents()
        components.scheme = "eddy-media"
        components.path = "play"
        components.queryItems = [
            URLQueryItem(name: "kind", value: kind == .embed ? "embed" : "video"),
            URLQueryItem(name: "src", value: src),
        ] + (poster.map { [URLQueryItem(name: "poster", value: $0)] } ?? [])
        return components.url
    }

    mutating func textBlock(_ id: String, _ runs: [RunView], font: NSFont, p: NSParagraphStyle, quote: Int) {
        let translated = translations[id]
        let start = out.length
        if mode == .translation, let translated {
            out.append(NSAttributedString(string: translated, attributes: base(font, p)))
        } else {
            out.append(inline(runs, font: font, color: .labelColor, p: p))
        }
        let range = NSRange(location: start, length: out.length - start)
        if quote > 0 { out.addAttribute(.eddyQuote, value: quote, range: range) }
        ranges.append((id, range))
        out.append(NSAttributedString(string: "\n", attributes: [.paragraphStyle: p]))
        if mode == .bilingual, translated != nil {
            translation(for: id, font: font, p: p)
        }
    }

    mutating func translation(for id: String, font: NSFont, p: NSParagraphStyle) {
        guard mode == .bilingual, let text = translations[id] else { return }
        translationRanges.append((id, NSRange(location: out.length, length: text.utf16.count)))
        out.append(NSAttributedString(string: text + "\n", attributes: base(font, p, color: .secondaryLabelColor)))
    }

    mutating func codeBlock(_ id: String, language: String?, text: String, highlights: [HighlightView], indent: CGFloat) {
        let p = NSMutableParagraphStyle()
        p.lineHeightMultiple = 1.15
        p.paragraphSpacing = style.fontSize * 1.1
        // Room above the code for the language label.
        p.paragraphSpacingBefore = CodeBlockMark.stripHeight
        p.headIndent = indent + 14
        p.firstLineHeadIndent = indent + 14
        p.tailIndent = -14
        let mark = CodeBlockMark(blockID: id, language: language)
        // One paragraph for the whole block, so its background draws as one.
        let joined = text.replacingOccurrences(of: "\n", with: "\u{2028}")
        let s = NSMutableAttributedString(string: joined, attributes: [
            .font: style.mono(), .foregroundColor: NSColor.labelColor, .paragraphStyle: p, .eddyCode: mark,
        ])
        let length = s.length
        for span in highlights where Int(span.end) <= length && span.start < span.end {
            s.addAttribute(.foregroundColor, value: CodeColors.color(span.class), range: NSRange(location: Int(span.start), length: Int(span.end - span.start)))
        }
        let start = out.length
        out.append(s)
        ranges.append((id, NSRange(location: start, length: length)))
        out.append(NSAttributedString(string: "\n", attributes: [.paragraphStyle: p, .eddyCode: mark]))
    }

    func inline(_ runs: [RunView], font: NSFont, color: NSColor, p: NSParagraphStyle) -> NSAttributedString {
        let s = NSMutableAttributedString()
        for run in runs {
            if run.math {
                let a = NSMutableAttributedString(attachment: MathRenderer.attachment(run.text, size: font.pointSize, display: false))
                a.addAttributes([.paragraphStyle: p, .font: font], range: NSRange(location: 0, length: a.length))
                s.append(a)
                continue
            }
            var f = font
            var attrs: [NSAttributedString.Key: Any] = [.paragraphStyle: p, .foregroundColor: color]
            if run.style & 4 != 0 {
                f = .monospacedSystemFont(ofSize: font.pointSize * 0.88, weight: .regular)
                attrs[.backgroundColor] = NSColor.quaternaryLabelColor.withAlphaComponent(0.25)
            }
            var traits: NSFontDescriptor.SymbolicTraits = []
            if run.style & 1 != 0 { traits.insert(.bold) }
            if run.style & 2 != 0 { traits.insert(.italic) }
            if !traits.isEmpty {
                let d = f.fontDescriptor.withSymbolicTraits(f.fontDescriptor.symbolicTraits.union(traits))
                f = NSFont(descriptor: d, size: f.pointSize) ?? f
            }
            if run.style & 128 != 0 { f = NSFont(descriptor: f.fontDescriptor, size: f.pointSize * 0.85) ?? f }
            if run.style & 32 != 0 { attrs[.baselineOffset] = f.pointSize * 0.35; f = NSFont(descriptor: f.fontDescriptor, size: f.pointSize * 0.7) ?? f }
            if run.style & 64 != 0 { attrs[.baselineOffset] = -f.pointSize * 0.15; f = NSFont(descriptor: f.fontDescriptor, size: f.pointSize * 0.7) ?? f }
            if run.style & 8 != 0 { attrs[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
            if run.style & 16 != 0 { attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue }
            if run.style & 256 != 0 { attrs[.backgroundColor] = NSColor.systemYellow.withAlphaComponent(0.3) }
            if let link = run.link, let url = URL(string: link) { attrs[.link] = url }
            attrs[.font] = f
            s.append(NSAttributedString(string: run.text, attributes: attrs))
        }
        return s
    }
}

enum CodeColors {
    static func color(_ c: TokenClassView) -> NSColor {
        switch c {
        case .keyword: .systemPink
        case .string: .systemRed
        case .comment: .secondaryLabelColor
        case .number, .constant: .systemPurple
        case .function: .systemTeal
        case .type, .tag: .systemBlue
        case .variable: .systemOrange
        case .attribute: .systemBrown
        case .operator, .punctuation: .labelColor
        }
    }
}
