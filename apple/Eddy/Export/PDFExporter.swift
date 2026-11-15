import AppKit
import CoreGraphics
import CoreText
import EddyKit
import ImageIO
import PDFKit

extension NSAttributedString.Key {
    fileprivate static let pdfInlineMath = NSAttributedString.Key("eddy.pdfInlineMath")
    fileprivate static let pdfPillColor = NSAttributedString.Key("eddy.pdfPillColor")
}

private final class InlineMathBox {
    let image: CGImage
    let size: CGSize
    let descent: CGFloat

    init(image: CGImage, size: CGSize, descent: CGFloat) {
        self.image = image
        self.size = size
        self.descent = descent
    }
}

/// Renders `PreparedPdfExportView` into a clean, print-ready A4 PDF with
/// vector CoreText typography, SwiftMath formulas, syntax-highlighted code
/// blocks, embedded images, bordered tables, page numbers, and PDF bookmarks.
enum PDFExporter {
    private static let pageWidth: CGFloat = 595.28
    private static let pageHeight: CGFloat = 841.89
    private static let marginLeft: CGFloat = 54
    private static let marginRight: CGFloat = 54
    private static let marginTop: CGFloat = 58
    private static let marginBottom: CGFloat = 56
    private static var columnWidth: CGFloat { pageWidth - marginLeft - marginRight }
    private static var topY: CGFloat { pageHeight - marginTop }
    private static var bottomY: CGFloat { marginBottom }
    private static var columnHeight: CGFloat { topY - bottomY }

    struct Bookmark {
        let title: String
        let level: Int
        let pageIndex: Int
        let yPoint: CGFloat
    }

    enum ExportRenderError: LocalizedError {
        case cancelled
        case contextCreationFailed

        var errorDescription: String? {
            switch self {
            case .cancelled: "Export canceled."
            case .contextCreationFailed: "Could not create PDF file at the chosen destination."
            }
        }
    }

    static func render(
        payload: PreparedPdfExportView,
        images: [String: Data],
        destination: URL,
        fonts: ReaderFonts,
        translatedRuns: @escaping (String) -> [RunView],
        isCancelled: @escaping () -> Bool
    ) throws {
        var caught: Error?
        let aqua = NSAppearance(named: .aqua) ?? NSAppearance.currentDrawing()
        aqua.performAsCurrentDrawingAppearance {
            do {
                try renderInLightAppearance(
                    payload: payload,
                    images: images,
                    destination: destination,
                    fonts: fonts,
                    translatedRuns: translatedRuns,
                    isCancelled: isCancelled
                )
            } catch {
                caught = error
            }
        }
        if let caught { throw caught }
    }

    private static func renderInLightAppearance(
        payload: PreparedPdfExportView,
        images: [String: Data],
        destination: URL,
        fonts: ReaderFonts,
        translatedRuns: @escaping (String) -> [RunView],
        isCancelled: @escaping () -> Bool
    ) throws {
        let style = PrintStyle(fonts: fonts)
        let decodedImages = decodeImages(images)

        var mediaBox = CGRect(x: 0, y: 0, width: pageWidth, height: pageHeight)
        var auxInfo: [CFString: Any] = [
            kCGPDFContextTitle: payload.title,
            kCGPDFContextCreator: "Eddy",
        ]
        if let author = payload.author, !author.isEmpty {
            auxInfo[kCGPDFContextAuthor] = author
        }

        guard let ctx = CGContext(destination as CFURL, mediaBox: &mediaBox, auxInfo as CFDictionary) else {
            throw ExportRenderError.contextCreationFailed
        }

        var writer = PageWriter(ctx: ctx, mediaBox: mediaBox, documentTitle: payload.title, style: style)
        writer.beginPage()

        let multiChapter = payload.chapters.count > 1

        for (chapterIdx, chapter) in payload.chapters.enumerated() {
            if isCancelled() {
                writer.finishCurrentPageIfOpen()
                ctx.closePDF()
                try? FileManager.default.removeItem(at: destination)
                throw ExportRenderError.cancelled
            }

            if chapterIdx > 0 {
                writer.newPage()
            }

            let items = buildChapterItems(
                chapter: chapter,
                mode: payload.mode,
                multiChapter: multiChapter,
                style: style,
                images: decodedImages,
                translatedRuns: translatedRuns
            )

            for item in items {
                if isCancelled() {
                    writer.finishCurrentPageIfOpen()
                    ctx.closePDF()
                    try? FileManager.default.removeItem(at: destination)
                    throw ExportRenderError.cancelled
                }
                writer.layout(item)
            }
        }

        writer.finishCurrentPageIfOpen()
        ctx.closePDF()

        if !writer.bookmarks.isEmpty {
            attachBookmarks(writer.bookmarks, to: destination)
        }
    }

    // MARK: - Typography & Inline Runs

    private struct PrintStyle {
        let baseSize: CGFloat = 11.5
        let lineHeight: CGFloat = 1.44
        /// The reader's typefaces.
        let fonts: ReaderFonts

        let textPrimary = NSColor(calibratedWhite: 0.11, alpha: 1.0)
        let textSecondary = NSColor(calibratedWhite: 0.36, alpha: 1.0)
        let textTertiary = NSColor(calibratedWhite: 0.52, alpha: 1.0)
        let accent = NSColor(calibratedRed: 0.05, green: 0.40, blue: 0.85, alpha: 1.0)
        let summaryAccent = NSColor(calibratedRed: 0.43, green: 0.34, blue: 0.81, alpha: 1.0)
        let linkColor = NSColor(calibratedRed: 0.04, green: 0.36, blue: 0.78, alpha: 1.0)
        let codeBg = NSColor(calibratedWhite: 0.95, alpha: 1.0)
        let borderColor = NSColor(calibratedWhite: 0.82, alpha: 1.0)

        func font(size: CGFloat? = nil, weight: NSFont.Weight = .regular) -> NSFont {
            fonts.latinFont(size: size ?? baseSize, weight: weight)
        }

        /// The Chinese in its own font, with the reader's spacing.
        func typeset(_ text: NSAttributedString) -> NSAttributedString {
            let s = NSMutableAttributedString(attributedString: text)
            fonts.typeset(s)
            return s
        }

        func mono(size: CGFloat? = nil) -> NSFont {
            .monospacedSystemFont(ofSize: size ?? (baseSize * 0.86), weight: .regular)
        }

        func paragraph(
            lineHeight: CGFloat? = nil,
            indent: CGFloat = 0,
            firstIndent: CGFloat? = nil,
            align: NSTextAlignment = .natural
        ) -> NSParagraphStyle {
            let p = NSMutableParagraphStyle()
            p.lineHeightMultiple = lineHeight ?? self.lineHeight
            p.paragraphSpacing = 0
            p.paragraphSpacingBefore = 0
            p.headIndent = indent
            p.firstLineHeadIndent = firstIndent ?? indent
            p.alignment = align
            if indent > 0 {
                p.tabStops = [NSTextTab(textAlignment: .left, location: indent)]
            }
            return p
        }
    }

    private enum LayoutItem {
        case heading(attr: NSAttributedString, bookmarkTitle: String?, bookmarkLevel: Int, quoteDepth: Int, spaceBefore: CGFloat, spaceAfter: CGFloat)
        case paragraph(attr: NSAttributedString, quoteDepth: Int, spaceBefore: CGFloat, spaceAfter: CGFloat)
        case summaryBox(tldr: NSAttributedString, points: [NSAttributedString])
        case codeBox(language: String?, lines: [NSAttributedString], quoteDepth: Int)
        case displayMath(image: CGImage, size: CGSize)
        case figure(image: CGImage?, size: CGSize, caption: NSAttributedString?)
        case table(rows: [[TableCellSpec]])
        case rule

        /// Whether it draws quote bars.
        var isQuoted: Bool {
            switch self {
            case let .heading(_, _, _, depth, _, _), let .paragraph(_, depth, _, _), let .codeBox(_, _, depth): depth > 0
            default: false
            }
        }
    }

    private struct TableCellSpec {
        let header: Bool
        let colspan: Int
        let attr: NSAttributedString
    }

    private static func distinctTranslation(_ map: [String: String], id: String, original: String) -> String? {
        guard let raw = map[id] else { return nil }
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed != original.trimmingCharacters(in: .whitespacesAndNewlines) else {
            return nil
        }
        return raw
    }

    private static func buildChapterItems(
        chapter: PreparedExportChapterView,
        mode: ExportContentModeView,
        multiChapter: Bool,
        style: PrintStyle,
        images: [String: (CGImage, CGSize)],
        translatedRuns: (String) -> [RunView]
    ) -> [LayoutItem] {
        var items: [LayoutItem] = []

        // Chapter / Article Title
        let titleFont = style.font(size: style.baseSize * 1.85, weight: .bold)
        let titlePara = style.paragraph(lineHeight: 1.15)
        let shownTitle: String = switch mode {
        case .translation:
            distinctTranslation(chapter.translations, id: "title", original: chapter.title) ?? chapter.title
        case .original, .bilingual:
            chapter.title
        }
        let titleAttr = NSAttributedString(
            string: shownTitle,
            attributes: [.font: titleFont, .foregroundColor: style.textPrimary, .paragraphStyle: titlePara]
        )
        let topBookmarkLevel = Int(chapter.chapterLevel) + 1
        items.append(.heading(
            attr: style.typeset(titleAttr),
            bookmarkTitle: multiChapter ? shownTitle : nil,
            bookmarkLevel: topBookmarkLevel,
            quoteDepth: 0,
            spaceBefore: 0,
            spaceAfter: 5
        ))

        if mode == .bilingual, let trTitle = distinctTranslation(chapter.translations, id: "title", original: chapter.title) {
            let subAttr = NSAttributedString(
                string: trTitle,
                attributes: [
                    .font: style.font(size: style.baseSize * 1.25, weight: .semibold),
                    .foregroundColor: style.textSecondary,
                    .paragraphStyle: style.paragraph(lineHeight: 1.2),
                ]
            )
            items.append(.paragraph(attr: style.typeset(subAttr), quoteDepth: 0, spaceBefore: 0, spaceAfter: 5))
        }

        // Metadata byline
        var meta: [String] = []
        if let author = chapter.author, !author.trimmingCharacters(in: .whitespaces).isEmpty {
            meta.append(author)
        }
        if let src = chapter.sourceTitle, !src.trimmingCharacters(in: .whitespaces).isEmpty, src != chapter.author {
            meta.append(src)
        }
        if chapter.publishedAt > 0 {
            let d = Date(timeIntervalSince1970: TimeInterval(chapter.publishedAt) / 1000)
            meta.append(d.formatted(date: .abbreviated, time: .omitted))
        }
        if !meta.isEmpty {
            let metaAttr = NSAttributedString(
                string: meta.joined(separator: " · "),
                attributes: [
                    .font: style.font(size: style.baseSize * 0.84),
                    .foregroundColor: style.textSecondary,
                    .paragraphStyle: style.paragraph(lineHeight: 1.2),
                ]
            )
            items.append(.paragraph(attr: style.typeset(metaAttr), quoteDepth: 0, spaceBefore: 0, spaceAfter: 14))
        }

        // Optional AI Summary card
        if let summary = chapter.summary {
            let tldrAttr = NSAttributedString(
                string: summary.tldr,
                attributes: [
                    .font: style.font(size: style.baseSize * 0.96, weight: .medium),
                    .foregroundColor: style.textPrimary,
                    .paragraphStyle: style.paragraph(lineHeight: 1.38),
                ]
            )
            let pointAttrs = summary.points.map { pt -> NSAttributedString in
                let p = style.paragraph(lineHeight: 1.35, indent: 12, firstIndent: 0)
                let s = NSMutableAttributedString(
                    string: "•\t",
                    attributes: [.font: style.font(size: style.baseSize * 0.92), .foregroundColor: style.textSecondary, .paragraphStyle: p]
                )
                s.append(NSAttributedString(
                    string: pt,
                    attributes: [.font: style.font(size: style.baseSize * 0.92), .foregroundColor: style.textPrimary, .paragraphStyle: p]
                ))
                return style.typeset(s)
            }
            items.append(.summaryBox(tldr: style.typeset(tldrAttr), points: pointAttrs))
        }

        let baseHeadingLevelOffset = multiChapter ? (topBookmarkLevel) : 0

        for block in chapter.blocks {
            let quote = Int(block.quoteDepth)
            let quoteIndent = CGFloat(quote) * 16

            switch block.kind {
            case let .heading(level, runs):
                let clamped = Int(min(max(level, 1), 6))
                let scales: [CGFloat] = [1.45, 1.28, 1.14, 1.05, 1.0, 1.0]
                let font = style.font(size: style.baseSize * scales[clamped - 1], weight: .bold)
                let p = style.paragraph(lineHeight: 1.22, indent: quoteIndent)
                let origPlain = runs.map(\.text).joined()
                let bookmarkTitle: String? = {
                    guard clamped <= 3 else { return nil }
                    let chosen = mode == .translation
                        ? (distinctTranslation(chapter.translations, id: block.id, original: origPlain) ?? origPlain)
                        : origPlain
                    let t = chosen.trimmingCharacters(in: .whitespacesAndNewlines)
                    return t.isEmpty ? nil : t
                }()
                let primaryRuns = (mode == .translation ? distinctTranslation(chapter.translations, id: block.id, original: origPlain).map(translatedRuns) : nil) ?? runs
                let attr = inlineAttributed(primaryRuns, font: font, color: style.textPrimary, p: p, style: style)
                items.append(.heading(
                    attr: attr,
                    bookmarkTitle: bookmarkTitle,
                    bookmarkLevel: baseHeadingLevelOffset + clamped,
                    quoteDepth: quote,
                    spaceBefore: clamped <= 2 ? 14 : 10,
                    spaceAfter: mode == .bilingual ? 3 : 6
                ))
                if mode == .bilingual, let tr = distinctTranslation(chapter.translations, id: block.id, original: origPlain) {
                    let trFont = style.font(size: font.pointSize * 0.9, weight: .semibold)
                    let trAttr = inlineAttributed(translatedRuns(tr), font: trFont, color: style.textSecondary, p: p, style: style)
                    items.append(.paragraph(attr: trAttr, quoteDepth: quote, spaceBefore: 0, spaceAfter: 6))
                }

            case let .paragraph(runs):
                let font = style.font()
                let p = style.paragraph(indent: quoteIndent)
                let origPlain = runs.map(\.text).joined()
                let primaryRuns = (mode == .translation ? distinctTranslation(chapter.translations, id: block.id, original: origPlain).map(translatedRuns) : nil) ?? runs
                let attr = inlineAttributed(primaryRuns, font: font, color: style.textPrimary, p: p, style: style)
                let trText = mode == .bilingual ? distinctTranslation(chapter.translations, id: block.id, original: origPlain) : nil
                items.append(.paragraph(attr: attr, quoteDepth: quote, spaceBefore: 0, spaceAfter: trText == nil ? 8 : 3))
                if let trText {
                    let trAttr = inlineAttributed(translatedRuns(trText), font: font, color: style.textSecondary, p: p, style: style)
                    items.append(.paragraph(attr: trAttr, quoteDepth: quote, spaceBefore: 0, spaceAfter: 9))
                }

            case let .listItem(ordered, depth, index, marker, runs):
                let indent = quoteIndent + CGFloat(depth + 1) * 20
                let p = style.paragraph(indent: indent, firstIndent: indent - 16)
                let font = style.font()
                let prefix = marker ? (ordered ? "\(index).\t" : "•\t") : "\t"
                let origPlain = runs.map(\.text).joined()
                let primaryRuns = (mode == .translation ? distinctTranslation(chapter.translations, id: block.id, original: origPlain).map(translatedRuns) : nil) ?? runs
                let combined = NSMutableAttributedString(
                    string: prefix,
                    attributes: [.font: font, .foregroundColor: style.textSecondary, .paragraphStyle: p]
                )
                combined.append(inlineAttributed(primaryRuns, font: font, color: style.textPrimary, p: p, style: style))
                let trText = mode == .bilingual ? distinctTranslation(chapter.translations, id: block.id, original: origPlain) : nil
                items.append(.paragraph(attr: combined, quoteDepth: quote, spaceBefore: 0, spaceAfter: trText == nil ? 4.5 : 2))
                if let trText {
                    let trP = style.paragraph(indent: indent)
                    let trAttr = inlineAttributed(translatedRuns(trText), font: font, color: style.textSecondary, p: trP, style: style)
                    items.append(.paragraph(attr: trAttr, quoteDepth: quote, spaceBefore: 0, spaceAfter: 5.5))
                }

            case let .code(language, text, highlights):
                let lines = buildCodeLines(text: text, highlights: highlights, style: style)
                items.append(.codeBox(language: language, lines: lines, quoteDepth: quote))

            case let .math(tex):
                if let (cg, ptSize) = renderMathImage(tex, fontSize: style.baseSize * 1.15, display: true) {
                    items.append(.displayMath(image: cg, size: ptSize))
                }

            case let .image(src, _, width, height, caption):
                let capID = "\(block.id)#caption"
                let capAttr: NSAttributedString? = {
                    guard !caption.isEmpty else { return nil }
                    let capFont = style.font(size: style.baseSize * 0.85)
                    let capPara = style.paragraph(lineHeight: 1.3, align: .center)
                    let origCap = caption.map(\.text).joined()
                    let s = NSMutableAttributedString()
                    if mode == .translation, let tr = distinctTranslation(chapter.translations, id: capID, original: origCap) {
                        s.append(inlineAttributed(translatedRuns(tr), font: capFont, color: style.textSecondary, p: capPara, style: style))
                    } else {
                        s.append(inlineAttributed(caption, font: capFont, color: style.textSecondary, p: capPara, style: style))
                        if mode == .bilingual, let tr = distinctTranslation(chapter.translations, id: capID, original: origCap) {
                            s.append(NSAttributedString(string: "\n", attributes: [.font: capFont, .paragraphStyle: capPara]))
                            s.append(inlineAttributed(translatedRuns(tr), font: capFont, color: style.textSecondary, p: capPara, style: style))
                        }
                    }
                    return s
                }()

                if let (cg, pxSize) = images[src] {
                    let natural = if let w = width, let h = height, w > 0, h > 0 {
                        CGSize(width: CGFloat(w), height: CGFloat(h))
                    } else {
                        pxSize
                    }
                    items.append(.figure(image: cg, size: natural, caption: capAttr))
                } else if let capAttr {
                    items.append(.figure(image: nil, size: .zero, caption: capAttr))
                }

            case let .table(rows):
                let cellFont = style.font(size: style.baseSize * 0.88)
                let headerFont = style.font(size: style.baseSize * 0.88, weight: .semibold)
                let cellPara = style.paragraph(lineHeight: 1.3)
                let builtRows: [[TableCellSpec]] = rows.enumerated().map { rIdx, row in
                    row.cells.enumerated().map { cIdx, cell in
                        let f = cell.header ? headerFont : cellFont
                        let segID = "\(block.id)#r\(rIdx)c\(cIdx)"
                        let origPlain = cell.runs.map(\.text).joined()
                        let attr = NSMutableAttributedString()
                        if mode == .translation, let tr = distinctTranslation(chapter.translations, id: segID, original: origPlain) {
                            attr.append(inlineAttributed(translatedRuns(tr), font: f, color: style.textPrimary, p: cellPara, style: style))
                        } else {
                            attr.append(inlineAttributed(cell.runs, font: f, color: style.textPrimary, p: cellPara, style: style))
                            if mode == .bilingual, let tr = distinctTranslation(chapter.translations, id: segID, original: origPlain) {
                                attr.append(NSAttributedString(string: "\n", attributes: [.font: cellFont, .paragraphStyle: cellPara]))
                                attr.append(inlineAttributed(translatedRuns(tr), font: cellFont, color: style.textSecondary, p: cellPara, style: style))
                            }
                        }
                        return TableCellSpec(header: cell.header, colspan: max(1, Int(cell.colspan)), attr: attr)
                    }
                }
                if !builtRows.isEmpty {
                    items.append(.table(rows: builtRows))
                }

            case let .media(kind, src, _, title):
                let label = (kind == .audio ? "♪ " : "▶︎ ") + (title ?? (kind == .embed ? "Embedded content" : kind == .video ? "Video" : "Audio"))
                var attrs: [NSAttributedString.Key: Any] = [
                    .font: style.font(weight: .medium),
                    .foregroundColor: style.linkColor,
                    .paragraphStyle: style.paragraph(indent: quoteIndent),
                ]
                if src.hasPrefix("http"), let url = URL(string: src) {
                    attrs[.link] = url
                }
                items.append(.paragraph(attr: style.typeset(NSAttributedString(string: label, attributes: attrs)), quoteDepth: quote, spaceBefore: 2, spaceAfter: 8))

            case .rule:
                items.append(.rule)
            }
        }

        return items
    }

    private static func inlineAttributed(
        _ runs: [RunView],
        font: NSFont,
        color: NSColor,
        p: NSParagraphStyle,
        style: PrintStyle
    ) -> NSAttributedString {
        let out = NSMutableAttributedString()
        for run in runs {
            if run.math {
                if let (cg, ptSize, descent) = renderInlineMath(run.text, fontSize: font.pointSize) {
                    out.append(makeInlineMathRun(cg: cg, size: ptSize, descent: descent, font: font, p: p))
                } else {
                    out.append(NSAttributedString(
                        string: run.text,
                        attributes: [.font: style.mono(size: font.pointSize * 0.88), .foregroundColor: style.textSecondary, .paragraphStyle: p]
                    ))
                }
                continue
            }
            guard !run.text.isEmpty else { continue }
            var f = font
            var fg = color
            var attrs: [NSAttributedString.Key: Any] = [.paragraphStyle: p]

            if run.style & 4 != 0 {
                f = .monospacedSystemFont(ofSize: font.pointSize * 0.88, weight: .regular)
                attrs[.pdfPillColor] = NSColor(calibratedWhite: 0.93, alpha: 1.0)
            }
            var traits: NSFontDescriptor.SymbolicTraits = []
            if run.style & 1 != 0 { traits.insert(.bold) }
            if run.style & 2 != 0 { traits.insert(.italic) }
            if !traits.isEmpty {
                let d = f.fontDescriptor.withSymbolicTraits(f.fontDescriptor.symbolicTraits.union(traits))
                f = NSFont(descriptor: d, size: f.pointSize) ?? f
            }
            if run.style & 128 != 0 {
                f = NSFont(descriptor: f.fontDescriptor, size: f.pointSize * 0.85) ?? f
            }
            if run.style & 32 != 0 {
                attrs[NSAttributedString.Key(kCTSuperscriptAttributeName as String)] = 1
                f = NSFont(descriptor: f.fontDescriptor, size: f.pointSize * 0.72) ?? f
            }
            if run.style & 64 != 0 {
                attrs[NSAttributedString.Key(kCTSuperscriptAttributeName as String)] = -1
                f = NSFont(descriptor: f.fontDescriptor, size: f.pointSize * 0.72) ?? f
            }
            if run.style & 8 != 0 {
                attrs[.strikethroughStyle] = NSUnderlineStyle.single.rawValue
            }
            if run.style & 16 != 0 {
                attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue
            }
            if run.style & 256 != 0 {
                attrs[.pdfPillColor] = NSColor(calibratedRed: 1.0, green: 0.94, blue: 0.65, alpha: 1.0)
            }
            if let link = run.link, (link.hasPrefix("http://") || link.hasPrefix("https://")), let url = URL(string: link) {
                attrs[.link] = url
                fg = style.linkColor
                attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue
            }
            attrs[.font] = f
            attrs[.foregroundColor] = fg
            out.append(NSAttributedString(string: run.text, attributes: attrs))
        }
        style.fonts.typeset(out)
        return out
    }

    private static func makeInlineMathRun(
        cg: CGImage,
        size: CGSize,
        descent: CGFloat,
        font: NSFont,
        p: NSParagraphStyle
    ) -> NSAttributedString {
        let box = InlineMathBox(image: cg, size: size, descent: descent)
        var callbacks = CTRunDelegateCallbacks(
            version: kCTRunDelegateCurrentVersion,
            dealloc: { ref in
                Unmanaged<InlineMathBox>.fromOpaque(ref).release()
            },
            getAscent: { ref in
                let b = Unmanaged<InlineMathBox>.fromOpaque(ref).takeUnretainedValue()
                return max(0, b.size.height - b.descent)
            },
            getDescent: { ref in
                let b = Unmanaged<InlineMathBox>.fromOpaque(ref).takeUnretainedValue()
                return b.descent
            },
            getWidth: { ref in
                let b = Unmanaged<InlineMathBox>.fromOpaque(ref).takeUnretainedValue()
                return b.size.width + 2
            }
        )
        let retained = Unmanaged.passRetained(box).toOpaque()
        let attrs: [NSAttributedString.Key: Any]
        if let delegate = CTRunDelegateCreate(&callbacks, retained) {
            attrs = [
                .font: font,
                .paragraphStyle: p,
                .pdfInlineMath: box,
                NSAttributedString.Key(kCTRunDelegateAttributeName as String): delegate,
            ]
        } else {
            Unmanaged<InlineMathBox>.fromOpaque(retained).release()
            attrs = [.font: font, .paragraphStyle: p]
        }
        return NSAttributedString(string: "\u{FFFC}", attributes: attrs)
    }

    private static func renderInlineMath(_ tex: String, fontSize: CGFloat) -> (CGImage, CGSize, CGFloat)? {
        let attachment = MathRenderer.attachment(tex, size: fontSize, display: false)
        guard let nsImage = attachment.image,
              let cg = nsImage.cgImage(forProposedRect: nil, context: nil, hints: nil)
        else { return nil }
        let maxW = columnWidth * 0.92
        var size = attachment.naturalSize
        var descent = attachment.descent
        if size.width > maxW, size.width > 0 {
            let scale = maxW / size.width
            size = CGSize(width: maxW, height: size.height * scale)
            descent *= scale
        }
        return (cg, size, descent)
    }

    private static func renderMathImage(_ tex: String, fontSize: CGFloat, display: Bool) -> (CGImage, CGSize)? {
        let attachment = MathRenderer.attachment(tex, size: fontSize, display: display)
        guard let nsImage = attachment.image,
              let cg = nsImage.cgImage(forProposedRect: nil, context: nil, hints: nil)
        else { return nil }
        var size = attachment.naturalSize
        if size.width > columnWidth, size.width > 0 {
            let scale = columnWidth / size.width
            size = CGSize(width: columnWidth, height: size.height * scale)
        }
        return (cg, size)
    }

    private static func buildCodeLines(
        text: String,
        highlights: [HighlightView],
        style: PrintStyle
    ) -> [NSAttributedString] {
        let font = style.mono()
        let p = style.paragraph(lineHeight: 1.32)
        let full = NSMutableAttributedString(
            string: text,
            attributes: [.font: font, .foregroundColor: style.textPrimary, .paragraphStyle: p]
        )
        let len = full.length
        for span in highlights where Int(span.end) <= len && span.start < span.end {
            full.addAttribute(
                .foregroundColor,
                value: CodeColors.color(span.class),
                range: NSRange(location: Int(span.start), length: Int(span.end - span.start))
            )
        }
        let ns = full.string as NSString
        var out: [NSAttributedString] = []
        var lineStart = 0
        while lineStart <= ns.length {
            let lineRange = ns.lineRange(for: NSRange(location: lineStart, length: 0))
            var contentLen = lineRange.length
            while contentLen > 0 {
                let ch = ns.character(at: lineRange.location + contentLen - 1)
                if ch == 10 || ch == 13 { contentLen -= 1 } else { break }
            }
            let sub = full.attributedSubstring(from: NSRange(location: lineRange.location, length: contentLen))
            out.append(sub.length > 0 ? sub : NSAttributedString(string: " ", attributes: [.font: font, .paragraphStyle: p]))
            if lineRange.length == 0 { break }
            lineStart = NSMaxRange(lineRange)
            if lineStart == ns.length { break }
        }
        if out.isEmpty {
            out.append(NSAttributedString(string: " ", attributes: [.font: font, .paragraphStyle: p]))
        }
        return out
    }

    private static func decodeImages(_ raw: [String: Data]) -> [String: (CGImage, CGSize)] {
        var out: [String: (CGImage, CGSize)] = [:]
        for (url, data) in raw {
            if let source = CGImageSourceCreateWithData(data as CFData, nil),
               CGImageSourceGetCount(source) >= 1,
               let cg = CGImageSourceCreateImageAtIndex(source, 0, nil)
            {
                out[url] = (cg, CGSize(width: cg.width, height: cg.height))
            } else if let nsImg = NSImage(data: data),
                      let cg = nsImg.cgImage(forProposedRect: nil, context: nil, hints: nil)
            {
                out[url] = (cg, nsImg.size)
            }
        }
        return out
    }

    private static func attachBookmarks(_ bookmarks: [Bookmark], to url: URL) {
        guard let doc = PDFDocument(url: url), doc.pageCount > 0 else { return }
        let root = PDFOutline()
        var stack: [(level: Int, node: PDFOutline)] = [(0, root)]

        for bm in bookmarks {
            guard let page = doc.page(at: min(max(0, bm.pageIndex), doc.pageCount - 1)) else { continue }
            let node = PDFOutline()
            node.label = bm.title
            node.destination = PDFDestination(page: page, at: NSPoint(x: marginLeft, y: bm.yPoint))

            while stack.count > 1 && stack.last!.level >= bm.level {
                stack.removeLast()
            }
            let parent = stack.last!.node
            parent.insertChild(node, at: parent.numberOfChildren)
            stack.append((bm.level, node))
        }

        doc.outlineRoot = root
        doc.write(to: url)
    }

    // MARK: - Page Layout Engine

    private struct PageWriter {
        let ctx: CGContext
        let mediaBox: CGRect
        let documentTitle: String
        let style: PrintStyle

        var pageIndex = -1
        var pageOpen = false
        var cursorY: CGFloat = 0
        /// Where the quote bars of the item before end, for the next
        /// quoted item to take them up: a quote has one bar, not one per
        /// paragraph.
        var quoteTail: (depth: Int, y: CGFloat, page: Int)?
        var bookmarks: [Bookmark] = []

        mutating func beginPage() {
            var box = mediaBox
            ctx.beginPage(mediaBox: &box)
            ctx.textMatrix = .identity
            pageIndex += 1
            pageOpen = true
            cursorY = PDFExporter.topY
            if pageIndex > 0 {
                drawRunningHeader()
            }
        }

        mutating func finishCurrentPageIfOpen() {
            guard pageOpen else { return }
            drawPageFooter()
            ctx.endPage()
            pageOpen = false
        }

        mutating func newPage() {
            finishCurrentPageIfOpen()
            beginPage()
        }

        private var remainingHeight: CGFloat {
            max(0, cursorY - PDFExporter.bottomY)
        }

        private func drawRunningHeader() {
            let attr = NSAttributedString(
                string: documentTitle,
                attributes: [
                    .font: style.font(size: 8.5),
                    .foregroundColor: style.textTertiary,
                ]
            )
            let line = CTLineCreateWithAttributedString(attr)
            let truncated = CTLineCreateTruncatedLine(
                line,
                Double(PDFExporter.columnWidth),
                .end,
                CTLineCreateWithAttributedString(NSAttributedString(string: "…", attributes: [.font: style.font(size: 8.5), .foregroundColor: style.textTertiary]))
            ) ?? line
            ctx.saveGState()
            ctx.textPosition = CGPoint(x: PDFExporter.marginLeft, y: PDFExporter.pageHeight - 36)
            CTLineDraw(truncated, ctx)
            ctx.restoreGState()
        }

        private func drawPageFooter() {
            let label = "\(pageIndex + 1)"
            let attr = NSAttributedString(
                string: label,
                attributes: [
                    .font: style.font(size: 9),
                    .foregroundColor: style.textTertiary,
                ]
            )
            let line = CTLineCreateWithAttributedString(attr)
            let width = CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
            ctx.saveGState()
            ctx.textPosition = CGPoint(x: (PDFExporter.pageWidth - width) / 2, y: 28)
            CTLineDraw(line, ctx)
            ctx.restoreGState()
        }

        mutating func layout(_ item: LayoutItem) {
            defer { if !item.isQuoted { quoteTail = nil } }
            switch item {
            case let .heading(attr, bookmarkTitle, bookmarkLevel, quoteDepth, spaceBefore, spaceAfter):
                if cursorY < PDFExporter.topY - 1 {
                    cursorY -= spaceBefore
                }
                let estHeight = measureHeight(attr, width: PDFExporter.columnWidth)
                // Keep heading attached to at least ~44 pt of following text.
                if remainingHeight < estHeight + 44 && cursorY < PDFExporter.topY - 1 {
                    newPage()
                }
                if let bookmarkTitle {
                    bookmarks.append(Bookmark(
                        title: bookmarkTitle,
                        level: bookmarkLevel,
                        pageIndex: pageIndex,
                        yPoint: min(PDFExporter.pageHeight - 24, cursorY + 10)
                    ))
                }
                drawFlowingText(attr, x: PDFExporter.marginLeft, width: PDFExporter.columnWidth, quoteDepth: quoteDepth)
                cursorY -= spaceAfter

            case let .paragraph(attr, quoteDepth, spaceBefore, spaceAfter):
                if cursorY < PDFExporter.topY - 1 {
                    cursorY -= spaceBefore
                }
                if remainingHeight < 22 {
                    newPage()
                }
                drawFlowingText(attr, x: PDFExporter.marginLeft, width: PDFExporter.columnWidth, quoteDepth: quoteDepth)
                cursorY -= spaceAfter

            case let .summaryBox(tldr, points):
                drawSummaryBox(tldr: tldr, points: points)

            case let .codeBox(language, lines, quoteDepth):
                drawCodeBox(language: language, lines: lines, quoteDepth: quoteDepth)

            case let .displayMath(image, size):
                let needed = size.height + 16
                if remainingHeight < needed && cursorY < PDFExporter.topY - 1 {
                    newPage()
                }
                cursorY -= 6
                let x = PDFExporter.marginLeft + (PDFExporter.columnWidth - size.width) / 2
                let rect = CGRect(x: x, y: cursorY - size.height, width: size.width, height: size.height)
                ctx.saveGState()
                ctx.draw(image, in: rect)
                ctx.restoreGState()
                cursorY -= size.height + 10

            case let .figure(image, naturalSize, caption):
                drawFigure(image: image, naturalSize: naturalSize, caption: caption)

            case let .table(rows):
                drawTable(rows: rows)

            case .rule:
                if remainingHeight < 28 { newPage() }
                cursorY -= 10
                ctx.saveGState()
                ctx.setStrokeColor(style.borderColor.cgColor)
                ctx.setLineWidth(0.75)
                let midY = cursorY - 4
                let ruleW: CGFloat = 120
                let startX = (PDFExporter.pageWidth - ruleW) / 2
                ctx.move(to: CGPoint(x: startX, y: midY))
                ctx.addLine(to: CGPoint(x: startX + ruleW, y: midY))
                ctx.strokePath()
                ctx.restoreGState()
                cursorY -= 18
            }
        }

        private func measureHeight(_ attr: NSAttributedString, width: CGFloat) -> CGFloat {
            guard attr.length > 0 else { return 0 }
            let fs = CTFramesetterCreateWithAttributedString(attr)
            let size = CTFramesetterSuggestFrameSizeWithConstraints(
                fs,
                CFRange(location: 0, length: attr.length),
                nil,
                CGSize(width: max(20, width), height: 10_000),
                nil
            )
            return ceil(size.height)
        }

        private mutating func drawFlowingText(
            _ attr: NSAttributedString,
            x: CGFloat,
            width: CGFloat,
            quoteDepth: Int
        ) {
            guard attr.length > 0 else { return }
            let framesetter = CTFramesetterCreateWithAttributedString(attr)
            var loc = 0
            let totalLen = attr.length

            while loc < totalLen {
                if remainingHeight < 18 {
                    newPage()
                }
                let availH = remainingHeight
                let frameRect = CGRect(x: x, y: cursorY - availH, width: width, height: availH)
                let path = CGPath(rect: frameRect, transform: nil)
                let frame = CTFramesetterCreateFrame(framesetter, CFRange(location: loc, length: 0), path, nil)
                let lines = CTFrameGetLines(frame) as? [CTLine] ?? []
                let vis = CTFrameGetVisibleStringRange(frame)

                if lines.isEmpty || vis.length == 0 {
                    if cursorY >= PDFExporter.topY - 1 {
                        // Force progress by at least one character to prevent an infinite loop.
                        loc += 1
                        continue
                    }
                    newPage()
                    continue
                }

                var origins = [CGPoint](repeating: .zero, count: lines.count)
                CTFrameGetLineOrigins(frame, CFRange(location: 0, length: 0), &origins)

                var firstAscent: CGFloat = 0
                CTLineGetTypographicBounds(lines[0], &firstAscent, nil, nil)
                var lastDescent: CGFloat = 0
                CTLineGetTypographicBounds(lines[lines.count - 1], nil, &lastDescent, nil)

                let topEdge = frameRect.minY + origins[0].y + firstAscent
                let bottomEdge = frameRect.minY + origins[lines.count - 1].y - lastDescent

                if quoteDepth > 0 {
                    ctx.saveGState()
                    ctx.setFillColor(style.accent.withAlphaComponent(0.55).cgColor)
                    for level in 0..<quoteDepth {
                        let barX = x + CGFloat(level) * 16 + 2
                        var top = max(bottomEdge + 3, topEdge + 1)
                        if loc == 0, let tail = quoteTail, tail.page == pageIndex, level < tail.depth { top = max(top, tail.y) }
                        ctx.fill(CGRect(x: barX, y: bottomEdge - 1, width: 2.5, height: top - (bottomEdge - 1)))
                    }
                    ctx.restoreGState()
                    quoteTail = (quoteDepth, bottomEdge - 1, pageIndex)
                }

                drawFrameDecorations(lines: lines, origins: origins, frameOrigin: frameRect.origin, beforeText: true)
                ctx.saveGState()
                ctx.textMatrix = .identity
                CTFrameDraw(frame, ctx)
                ctx.restoreGState()
                drawFrameDecorations(lines: lines, origins: origins, frameOrigin: frameRect.origin, beforeText: false)

                cursorY = bottomEdge
                loc += vis.length
                if loc < totalLen {
                    newPage()
                }
            }
        }

        private func drawFrameDecorations(
            lines: [CTLine],
            origins: [CGPoint],
            frameOrigin: CGPoint,
            beforeText: Bool
        ) {
            for (line, origin) in zip(lines, origins) {
                let lineOrigin = CGPoint(x: frameOrigin.x + origin.x, y: frameOrigin.y + origin.y)
                guard let runs = CTLineGetGlyphRuns(line) as? [CTRun] else { continue }
                for run in runs {
                    let attrs = CTRunGetAttributes(run) as NSDictionary
                    let range = CTRunGetStringRange(run)
                    let xOffset = CTLineGetOffsetForStringIndex(line, range.location, nil)
                    var ascent: CGFloat = 0
                    var descent: CGFloat = 0
                    let width = CGFloat(CTRunGetTypographicBounds(run, CFRange(location: 0, length: 0), &ascent, &descent, nil))

                    if beforeText, let pill = attrs[NSAttributedString.Key.pdfPillColor] as? NSColor, width > 0 {
                        let rect = CGRect(
                            x: lineOrigin.x + xOffset - 1.5,
                            y: lineOrigin.y - descent - 0.5,
                            width: width + 3,
                            height: ascent + descent + 1
                        )
                        ctx.saveGState()
                        ctx.setFillColor(pill.cgColor)
                        ctx.addPath(CGPath(roundedRect: rect, cornerWidth: 2.5, cornerHeight: 2.5, transform: nil))
                        ctx.fillPath()
                        ctx.restoreGState()
                    } else if !beforeText {
                        if let math = attrs[NSAttributedString.Key.pdfInlineMath] as? InlineMathBox {
                            let drawRect = CGRect(
                                x: lineOrigin.x + xOffset + 1,
                                y: lineOrigin.y - math.descent,
                                width: math.size.width,
                                height: math.size.height
                            )
                            ctx.saveGState()
                            ctx.draw(math.image, in: drawRect)
                            ctx.restoreGState()
                        }
                        if let linkURL = (attrs[NSAttributedString.Key.link] as? URL) ?? (attrs["NSLink"] as? URL), width > 0 {
                            let linkRect = CGRect(
                                x: lineOrigin.x + xOffset,
                                y: lineOrigin.y - descent,
                                width: width,
                                height: ascent + descent
                            )
                            ctx.setURL(linkURL as CFURL, for: linkRect)
                        }
                    }
                }
            }
        }

        private mutating func drawSummaryBox(tldr: NSAttributedString, points: [NSAttributedString]) {
            let padX: CGFloat = 14
            let padY: CGFloat = 12
            let innerW = PDFExporter.columnWidth - padX * 2
            let headerAttr = NSAttributedString(
                string: "SUMMARY",
                attributes: [
                    .font: NSFont.systemFont(ofSize: 9, weight: .bold),
                    .foregroundColor: style.summaryAccent,
                    .kern: 0.7,
                ]
            )
            let headerH: CGFloat = 15
            let tldrH = measureHeight(tldr, width: innerW)
            let pointsH = points.reduce(CGFloat(0)) { $0 + measureHeight($1, width: innerW) + 4 }
            let totalH = padY * 2 + headerH + tldrH + (points.isEmpty ? 0 : pointsH + 4)

            if remainingHeight < min(totalH, PDFExporter.columnHeight * 0.8) && cursorY < PDFExporter.topY - 1 {
                newPage()
            }

            let boxTop = cursorY
            let boxRect = CGRect(x: PDFExporter.marginLeft, y: boxTop - totalH, width: PDFExporter.columnWidth, height: totalH)
            ctx.saveGState()
            ctx.setFillColor(NSColor(calibratedRed: 0.96, green: 0.96, blue: 0.985, alpha: 1.0).cgColor)
            ctx.addPath(CGPath(roundedRect: boxRect, cornerWidth: 6, cornerHeight: 6, transform: nil))
            ctx.fillPath()
            ctx.setFillColor(style.summaryAccent.cgColor)
            ctx.fill(CGRect(x: boxRect.minX, y: boxRect.minY + 4, width: 3, height: max(4, boxRect.height - 8)))
            ctx.restoreGState()

            cursorY -= padY
            let hLine = CTLineCreateWithAttributedString(headerAttr)
            ctx.saveGState()
            ctx.textPosition = CGPoint(x: PDFExporter.marginLeft + padX, y: cursorY - 9)
            CTLineDraw(hLine, ctx)
            ctx.restoreGState()
            cursorY -= headerH

            drawFlowingText(tldr, x: PDFExporter.marginLeft + padX, width: innerW, quoteDepth: 0)
            if !points.isEmpty {
                cursorY -= 6
                for pt in points {
                    drawFlowingText(pt, x: PDFExporter.marginLeft + padX, width: innerW, quoteDepth: 0)
                    cursorY -= 3.5
                }
            }
            cursorY = min(cursorY - padY + 4, boxRect.minY) - 14
        }

        private mutating func drawCodeBox(language: String?, lines: [NSAttributedString], quoteDepth: Int) {
            let indent = CGFloat(quoteDepth) * 16
            let boxX = PDFExporter.marginLeft + indent
            let boxW = PDFExporter.columnWidth - indent
            let padX: CGFloat = 11
            let innerW = max(60, boxW - padX * 2)
            let stripH: CGFloat = (language?.isEmpty == false && !CodeBlockMark.isPlain(language)) ? 18 : 8
            let padBottom: CGFloat = 9

            let lineHeights = lines.map { max(13, measureHeight($0, width: innerW)) }
            let totalCodeH = stripH + lineHeights.reduce(0, +) + padBottom

            if totalCodeH <= PDFExporter.columnHeight * 0.72 && remainingHeight < totalCodeH && cursorY < PDFExporter.topY - 1 {
                newPage()
            } else if remainingHeight < 44 {
                newPage()
            }

            if cursorY < PDFExporter.topY - 1 { cursorY -= 4 }

            var lineIdx = 0
            var firstSlice = true
            while lineIdx < lines.count {
                if remainingHeight < 36 { newPage() }
                let topPad = firstSlice ? stripH : 9
                var sliceEnd = lineIdx
                var usedH = topPad + padBottom
                while sliceEnd < lines.count {
                    let h = lineHeights[sliceEnd]
                    if sliceEnd > lineIdx && usedH + h > remainingHeight {
                        break
                    }
                    usedH += h
                    sliceEnd += 1
                }

                let boxRect = CGRect(x: boxX, y: cursorY - usedH, width: boxW, height: usedH)
                ctx.saveGState()
                ctx.setFillColor(style.codeBg.cgColor)
                ctx.addPath(CGPath(roundedRect: boxRect, cornerWidth: 6, cornerHeight: 6, transform: nil))
                ctx.fillPath()
                if quoteDepth > 0 {
                    ctx.setFillColor(style.accent.withAlphaComponent(0.55).cgColor)
                    for level in 0..<quoteDepth {
                        var top = boxRect.maxY
                        if firstSlice, let tail = quoteTail, tail.page == pageIndex, level < tail.depth { top = max(top, tail.y) }
                        ctx.fill(CGRect(x: PDFExporter.marginLeft + CGFloat(level) * 16 + 2, y: boxRect.minY, width: 2.5, height: top - boxRect.minY))
                    }
                    quoteTail = (quoteDepth, boxRect.minY, pageIndex)
                }
                ctx.restoreGState()

                if firstSlice, stripH > 10, let language {
                    let label = NSAttributedString(
                        string: CodeBlockMark.displayName(language),
                        attributes: [.font: NSFont.systemFont(ofSize: 8.5, weight: .medium), .foregroundColor: style.textTertiary]
                    )
                    let ctLine = CTLineCreateWithAttributedString(label)
                    let lw = CGFloat(CTLineGetTypographicBounds(ctLine, nil, nil, nil))
                    ctx.saveGState()
                    ctx.textPosition = CGPoint(x: boxRect.maxX - padX - lw, y: boxRect.maxY - 13)
                    CTLineDraw(ctLine, ctx)
                    ctx.restoreGState()
                }

                cursorY -= topPad
                for i in lineIdx..<sliceEnd {
                    drawFlowingText(lines[i], x: boxX + padX, width: innerW, quoteDepth: 0)
                }
                cursorY = boxRect.minY
                lineIdx = sliceEnd
                firstSlice = false
                if lineIdx < lines.count {
                    newPage()
                }
            }
            cursorY -= 10
        }

        private mutating func drawFigure(image: CGImage?, naturalSize: CGSize, caption: NSAttributedString?) {
            let captionH = caption.map { measureHeight($0, width: PDFExporter.columnWidth * 0.9) + 6 } ?? 0
            var imgSize: CGSize = .zero
            if image != nil, naturalSize.width > 0, naturalSize.height > 0 {
                let maxW = PDFExporter.columnWidth
                let maxH = min(PDFExporter.columnHeight * 0.65, PDFExporter.columnHeight - captionH - 20)
                let scale = min(1.0, min(maxW / naturalSize.width, max(80, maxH) / naturalSize.height))
                imgSize = CGSize(width: naturalSize.width * scale, height: naturalSize.height * scale)
            }
            let totalNeeded = imgSize.height + captionH + 14
            if remainingHeight < totalNeeded && cursorY < PDFExporter.topY - 1 {
                newPage()
            }
            if cursorY < PDFExporter.topY - 1 { cursorY -= 6 }

            if let image, imgSize.width > 0 {
                let x = PDFExporter.marginLeft + (PDFExporter.columnWidth - imgSize.width) / 2
                let rect = CGRect(x: x, y: cursorY - imgSize.height, width: imgSize.width, height: imgSize.height)
                ctx.saveGState()
                ctx.draw(image, in: rect)
                ctx.restoreGState()
                cursorY -= imgSize.height + 5
            }

            if let caption {
                let capW = PDFExporter.columnWidth * 0.9
                let capX = PDFExporter.marginLeft + (PDFExporter.columnWidth - capW) / 2
                drawFlowingText(caption, x: capX, width: capW, quoteDepth: 0)
            }
            cursorY -= 10
        }

        private mutating func drawTable(rows: [[TableCellSpec]]) {
            let maxCols = rows.map { $0.reduce(0) { $0 + $1.colspan } }.max() ?? 1
            guard maxCols > 0 else { return }
            let colW = PDFExporter.columnWidth / CGFloat(maxCols)
            let padX: CGFloat = 6
            let padY: CGFloat = 5

            if remainingHeight < 48 { newPage() }
            if cursorY < PDFExporter.topY - 1 { cursorY -= 4 }

            for row in rows {
                var cellWidths: [CGFloat] = []
                var rowH: CGFloat = 20
                for cell in row {
                    let w = colW * CGFloat(cell.colspan)
                    cellWidths.append(w)
                    let h = measureHeight(cell.attr, width: max(24, w - padX * 2)) + padY * 2
                    rowH = max(rowH, min(h, PDFExporter.columnHeight * 0.85))
                }

                if remainingHeight < rowH && cursorY < PDFExporter.topY - 1 {
                    newPage()
                }

                let rowTop = cursorY
                let rowBottom = rowTop - rowH
                var colX = PDFExporter.marginLeft

                for (cell, w) in zip(row, cellWidths) {
                    let cellRect = CGRect(x: colX, y: rowBottom, width: w, height: rowH)
                    ctx.saveGState()
                    if cell.header {
                        ctx.setFillColor(style.codeBg.cgColor)
                        ctx.fill(cellRect)
                    }
                    ctx.setStrokeColor(style.borderColor.cgColor)
                    ctx.setLineWidth(0.6)
                    ctx.stroke(cellRect)
                    ctx.restoreGState()

                    if cell.attr.length > 0 {
                        let innerRect = CGRect(
                            x: colX + padX,
                            y: rowBottom + padY,
                            width: max(16, w - padX * 2),
                            height: max(10, rowH - padY * 2)
                        )
                        let fs = CTFramesetterCreateWithAttributedString(cell.attr)
                        let frame = CTFramesetterCreateFrame(fs, CFRange(location: 0, length: 0), CGPath(rect: innerRect, transform: nil), nil)
                        let lines = CTFrameGetLines(frame) as? [CTLine] ?? []
                        var origins = [CGPoint](repeating: .zero, count: lines.count)
                        CTFrameGetLineOrigins(frame, CFRange(location: 0, length: 0), &origins)
                        drawFrameDecorations(lines: lines, origins: origins, frameOrigin: innerRect.origin, beforeText: true)
                        ctx.saveGState()
                        ctx.textMatrix = .identity
                        CTFrameDraw(frame, ctx)
                        ctx.restoreGState()
                        drawFrameDecorations(lines: lines, origins: origins, frameOrigin: innerRect.origin, beforeText: false)
                    }

                    colX += w
                }
                cursorY = rowBottom
            }
            cursorY -= 10
        }
    }
}
