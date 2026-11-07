import Foundation
import PDFKit

/// Reads PDF text and metadata with PDFKit, so imported PDFs are searchable
/// and can be summarized, translated and read as reflowed text.
public final class PDFKitTextExtractor: PdfTextExtractor, @unchecked Sendable {
    public init() {}

    public func extract(path: String) -> PdfTextView? {
        guard let document = PDFDocument(url: URL(fileURLWithPath: path)) else { return nil }
        let attributes = document.documentAttributes ?? [:]
        let pages = (0..<document.pageCount).map { document.page(at: $0)?.string ?? "" }
        return PdfTextView(
            title: attributes[PDFDocumentAttribute.titleAttribute] as? String,
            author: attributes[PDFDocumentAttribute.authorAttribute] as? String,
            pages: pages
        )
    }
}
