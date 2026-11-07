import EddyKit
import SwiftUI

extension ItemView: @retroactive Identifiable {}
extension SeriesView: @retroactive Identifiable {}
extension SourceView: @retroactive Identifiable {}
extension TagView: @retroactive Identifiable {}

extension ItemView {
    /// Rough reading time: CJK at ~400 chars/min, Latin at ~1200 chars/min.
    var readingMinutes: Int {
        let perMinute = (lang == "cmn" || lang == "jpn") ? 400.0 : 1200.0
        return max(1, Int((Double(textLen) / perMinute).rounded()))
    }

    var minutesLeft: Int { max(1, Int(Double(readingMinutes) * (1 - progress))) }

    var date: Date { Date(timeIntervalSince1970: TimeInterval(publishedAt) / 1000) }

    var host: String {
        if format == .pdf { return "PDF" }
        if let host = url.flatMap({ URL(string: $0)?.host() }) { return host.replacingOccurrences(of: "www.", with: "") }
        return kind == .chapter ? "Book" : "Saved"
    }

    var sourceName: String { kind == .feed ? (sourceTitle ?? host) : host }

    var inProgress: Bool { progress > 0.02 && !finished }
}

struct ProgressRing: View {
    let value: Double
    var size: CGFloat = 14

    var body: some View {
        ZStack {
            Circle().stroke(Color.secondary.opacity(0.25), lineWidth: 2)
            Circle().trim(from: 0, to: value).stroke(Color.accentColor, style: StrokeStyle(lineWidth: 2, lineCap: .round)).rotationEffect(.degrees(-90))
        }
        .frame(width: size, height: size)
    }
}

struct ArticleRow: View {
    let item: ItemView

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Circle().fill(item.read ? Color.clear : Color.accentColor).frame(width: 7, height: 7)
                Text(item.sourceName).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                Spacer()
                if item.starred { Image(systemName: "star.fill").font(.caption2).foregroundStyle(.yellow) }
                if item.inProgress { ProgressRing(value: item.progress, size: 12) }
                Text(item.date, format: .relative(presentation: .named)).font(.caption).foregroundStyle(.tertiary)
            }
            Text(item.title).font(.body.weight(item.read ? .regular : .semibold)).lineLimit(2)
            if let excerpt = item.excerpt, !excerpt.isEmpty {
                Text(excerpt).font(.callout).foregroundStyle(.secondary).lineLimit(2)
            }
        }
        .padding(.vertical, 4)
    }
}

/// A book cover: the book's own cover or a PDF's first page as is; else a
/// gradient keyed to the title, with the lead image when there is one.
struct BookCover: View {
    @Environment(AppModel.self) private var model
    let title: String
    let subtitle: String?
    var imageItemID: String?
    /// The book's own cover image.
    var imageURL: String?
    var pdfItemID: String?
    var symbol: String = "book.closed"

    @State private var image: String?
    @State private var cover: NSImage?

    var body: some View {
        ZStack(alignment: .bottomLeading) {
            if let cover {
                Color.white.overlay { Image(nsImage: cover).resizable().aspectRatio(contentMode: .fill) }
            } else {
                LinearGradient(colors: palette, startPoint: .topLeading, endPoint: .bottomTrailing)
                if let image {
                    Color.clear.overlay { EngineImage(url: image, contentMode: .fill) }
                    LinearGradient(colors: [.clear, .black.opacity(0.65)], startPoint: .center, endPoint: .bottom)
                }
                VStack(alignment: .leading, spacing: 4) {
                    Image(systemName: symbol).font(.caption).opacity(0.8)
                    Spacer(minLength: 0)
                    Text(title)
                        .font(.system(size: 13, weight: .bold, design: .serif))
                        .lineLimit(4)
                    if let subtitle {
                        Text(subtitle).font(.system(size: 10, weight: .medium)).opacity(0.85).lineLimit(1)
                    }
                }
                .foregroundStyle(.white)
                .padding(10)
                .shadow(color: .black.opacity(0.35), radius: 2)
            }
            Rectangle().fill(Color.black.opacity(0.18)).frame(width: 5).frame(maxHeight: .infinity, alignment: .leading)
        }
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .task(id: imageItemID) {
            if let imageItemID, imageURL == nil, pdfItemID == nil { image = await model.leadImage(imageItemID) }
        }
        .task(id: imageURL ?? pdfItemID) {
            if let imageURL {
                cover = await model.image(imageURL)
            } else if let pdfItemID {
                cover = await model.pdfCover(pdfItemID)
            }
        }
    }

    private var palette: [Color] {
        let hue = Double(title.unicodeScalars.reduce(7) { ($0 &* 31 &+ Int($1.value)) % 360 }) / 360
        return [Color(hue: hue, saturation: 0.45, brightness: 0.62), Color(hue: (hue + 0.07).truncatingRemainder(dividingBy: 1), saturation: 0.55, brightness: 0.36)]
    }
}

struct EngineImage: View {
    @Environment(AppModel.self) private var model
    let url: String
    var contentMode: ContentMode = .fit
    @State private var image: NSImage?

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image).resizable().aspectRatio(contentMode: contentMode)
            } else {
                Color.clear
            }
        }
        .task(id: url) { image = await model.image(url) }
    }
}

struct MessageBanner: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let message = model.message {
            HStack(spacing: 10) {
                Image(systemName: "info.circle")
                Text(message).lineLimit(2)
                Button("OK") { model.message = nil }.buttonStyle(.borderless)
            }
            .font(.callout)
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(.regularMaterial, in: Capsule())
            .shadow(color: .black.opacity(0.1), radius: 8, y: 2)
            .padding(20)
            .task(id: message) {
                try? await Task.sleep(for: .seconds(6))
                if model.message == message { model.message = nil }
            }
        }
    }
}
