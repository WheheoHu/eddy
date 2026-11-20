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

    /// Read, started or finished: "Mark as Unread" has something to reset.
    var hasBeenRead: Bool { read || finished || progress > 0 || lastReadAt != nil }

    /// Read and finished: "Mark as Read" has nothing left to mark.
    var isDone: Bool { read && finished }
}

struct ProgressRing: View {
    let value: Double
    var size: CGFloat = 14

    var body: some View {
        ZStack {
            Circle().stroke(Theme.outline, lineWidth: 2)
            Circle().trim(from: 0, to: value).stroke(Theme.accent, style: StrokeStyle(lineWidth: 2, lineCap: .round)).rotationEffect(.degrees(-90))
        }
        .frame(width: size, height: size)
    }
}

/// The icon of a subscription's site, in colour, over whatever stands in
/// for it until it is here (or for good, when the site has none).
///
/// Both are always in the view and trade opacity: a list's rows must not
/// change the kind of their views as state changes. The icon is this
/// view's own state, so one arriving redraws the icon and not the list.
struct FeedIcon<Placeholder: View>: View {
    @Environment(AppModel.self) private var model
    /// `nil` for a row that is not a subscription's: only the placeholder.
    let sourceID: String?
    var size: CGFloat = 16
    /// Without an icon the view takes no room, and `gap` after it none:
    /// for a line of text that an icon may or may not lead.
    var collapses = false
    var gap: CGFloat = 0
    @ViewBuilder var placeholder: Placeholder
    @State private var loaded: Loaded?

    @Environment(\.colorScheme) private var scheme

    private struct Loaded {
        let id: String
        let icon: SiteIcon?
    }

    private struct Ask: Hashable {
        let id: String?
        let epoch: Int
    }

    private static var blank: NSImage { NSImage(size: NSSize(width: 1, height: 1)) }

    var body: some View {
        // A row reused for another feed shows that feed's icon or none,
        // never the last one's.
        let icon = sourceID.flatMap { id in loaded?.id == id ? loaded?.icon : model.cachedFeedIcon(id) }
        let away = collapses && icon == nil
        // A black drawing on a dark sidebar, a white one on a light page.
        let plate: Color? = switch (icon?.ink, scheme) {
        case (.dark, .dark): Color.white.opacity(0.88)
        case (.light, .light): Color.black.opacity(0.72)
        default: nil
        }
        ZStack {
            placeholder.opacity(icon == nil ? 1 : 0)
            Image(nsImage: icon?.image ?? Self.blank)
                .resizable()
                .interpolation(.high)
                .aspectRatio(contentMode: .fit)
                .padding(plate == nil ? 0 : size * 0.1)
                .frame(width: away ? 0 : size, height: size)
                .background(plate ?? Color.clear)
                .clipShape(RoundedRectangle(cornerRadius: size * 0.22, style: .continuous))
                .opacity(icon == nil ? 0 : 1)
        }
        .padding(.trailing, away ? 0 : gap)
        .accessibilityHidden(true)
        .task(id: Ask(id: sourceID, epoch: model.feedIconEpoch)) {
            guard let sourceID else { return }
            let icon = await model.feedIcon(sourceID)
            if !Task.isCancelled { loaded = Loaded(id: sourceID, icon: icon) }
        }
    }
}

extension FeedIcon where Placeholder == EmptyView {
    init(sourceID: String?, size: CGFloat = 16, collapses: Bool = false, gap: CGFloat = 0) {
        self.init(sourceID: sourceID, size: size, collapses: collapses, gap: gap) { EmptyView() }
    }
}

struct ArticleRow: View {
    let item: ItemView
    /// A finished article shows a check where its progress was: in
    /// Continue Reading, where it stays listed until the list starts over.
    var marksFinished = false

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            // Unread: a small square in the accent colour.
            RoundedRectangle(cornerRadius: 2)
                .fill(item.read ? Color.clear : Theme.accent)
                .frame(width: 7, height: 7)
                .padding(.top, 5)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    HStack(spacing: 0) {
                        // The feed's icon, when its site has one.
                        FeedIcon(sourceID: item.kind == .feed ? item.sourceId : nil, size: 14, collapses: true, gap: 5)
                        Text(item.sourceName).font(.system(size: 12, weight: .semibold)).foregroundStyle(Theme.text2).lineLimit(1)
                    }
                    if item.starred { Image(systemName: "star.fill").font(.system(size: 10)).foregroundStyle(Theme.star) }
                    let done = marksFinished && item.finished
                    if item.inProgress || done {
                        // One view either way: a row's views don't change
                        // kind as it is read.
                        ZStack {
                            ProgressRing(value: item.progress, size: 12).opacity(done ? 0 : 1)
                            Image(systemName: "checkmark")
                                .font(.system(size: 10, weight: .semibold))
                                .foregroundStyle(Theme.ok)
                                .opacity(done ? 1 : 0)
                        }
                        .accessibilityLabel(done ? Text("Finished") : Text("Reading"))
                    }
                    Spacer(minLength: 4)
                    Text(item.date, format: .relative(presentation: .named)).font(.system(size: 12)).foregroundStyle(Theme.text3).lineLimit(1)
                }
                Text(item.title)
                    .font(.system(size: 15, weight: item.read ? .regular : .semibold))
                    .foregroundStyle(item.read ? Theme.text2 : Theme.text1)
                    .lineLimit(2)
                if let excerpt = item.excerpt, !excerpt.isEmpty {
                    Text(excerpt).font(.system(size: 13)).foregroundStyle(Theme.text2).lineLimit(2)
                }
            }
        }
        .padding(.vertical, 11)
        .overlay(alignment: .bottom) { Hairline().padding(.leading, 15) }
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
    /// A subscription's cover is made from its site's icon, when it has one.
    var feedID: String?

    @State private var image: String?
    @State private var cover: NSImage?
    @State private var loadedIcon: SiteIcon?

    var body: some View {
        // In hand already, or just loaded.
        let siteIcon = loadedIcon ?? feedID.flatMap { model.cachedFeedIcon($0) }
        ZStack(alignment: .bottomLeading) {
            if let cover {
                Color.white.overlay { Image(nsImage: cover).resizable().aspectRatio(contentMode: .fill) }
            } else if let siteIcon {
                // A favicon is a few dozen pixels: it is not blown up to
                // fill the cover. It sits near its own size on a soft field
                // of its colours, over the name.
                let ink: Color = siteIcon.backdropIsLight ? Color(white: 0.13) : .white
                Color.clear.overlay { Image(nsImage: siteIcon.backdrop).resizable().interpolation(.high) }
                (siteIcon.backdropIsLight ? Color.white.opacity(0.22) : Color.black.opacity(0.22))
                VStack(alignment: .leading, spacing: 4) {
                    let side = min(44, max(24, CGFloat(siteIcon.pixels) / 2))
                    Image(nsImage: siteIcon.image)
                        .resizable()
                        .interpolation(.high)
                        .aspectRatio(contentMode: .fit)
                        .frame(width: side, height: side)
                        .clipShape(RoundedRectangle(cornerRadius: side * 0.22, style: .continuous))
                        .padding(.leading, 5)
                        .padding(.top, 4)
                    Spacer(minLength: 0)
                    Text(title)
                        .font(.system(size: 13, weight: .bold, design: .serif))
                        .lineLimit(4)
                    if let subtitle {
                        Text(subtitle).font(.system(size: 10, weight: .medium)).opacity(0.85).lineLimit(1)
                    }
                }
                .foregroundStyle(ink)
                .padding(10)
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
        .task(id: feedID) {
            loadedIcon = nil
            if let feedID { loadedIcon = await model.feedIcon(feedID) }
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

/// The notice along the bottom of the window. A saved page offers to show
/// itself; an error copies when clicked, and stays a little longer.
struct MessageBanner: View {
    @Environment(AppModel.self) private var model
    @State private var copied = false

    var body: some View {
        if let notice = model.notice {
            HStack(spacing: 10) {
                Image(systemName: notice.kind == .error ? "exclamationmark.triangle" : "info.circle")
                    .foregroundStyle(notice.kind == .error ? AnyShapeStyle(.orange) : AnyShapeStyle(.primary))
                Text(notice.text).lineLimit(2)
                switch notice.kind {
                case .info:
                    Button("OK") { model.notice = nil }
                case .error:
                    Button(copied ? "Copied" : "Copy") { copy(notice) }
                        .help("Copy the error")
                    Button { model.notice = nil } label: { Image(systemName: "xmark") }
                        .help("Dismiss")
                case let .saved(itemID):
                    Button("Show") {
                        model.notice = nil
                        model.reveal(itemID: itemID)
                    }
                    .help("Show it in Saved")
                case let .subscribed(sourceID):
                    Button("Show") {
                        model.notice = nil
                        model.show(feed: sourceID)
                    }
                    .help("Show the subscription")
                case let .exported(path):
                    Button("Show in Finder") {
                        model.notice = nil
                        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
                    }
                    .help("Reveal the exported file in Finder")
                }
            }
            .buttonStyle(.borderless)
            .font(.callout)
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(Theme.list, in: RoundedRectangle(cornerRadius: Theme.Radius.panel, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: Theme.Radius.panel, style: .continuous).strokeBorder(Theme.outline))
            .contentShape(Rectangle())
            .onTapGesture { if notice.kind == .error { copy(notice) } }
            .shadow(color: .black.opacity(0.1), radius: 8, y: 2)
            .padding(20)
            .task(id: notice.token) {
                copied = false
                try? await Task.sleep(for: .seconds(notice.kind == .error ? 12 : 6))
                if model.notice?.token == notice.token { model.notice = nil }
            }
        }
    }

    private func copy(_ notice: Notice) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(notice.text, forType: .string)
        copied = true
    }
}
