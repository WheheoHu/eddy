import EddyKit
import Foundation

struct SeedProgress: Equatable {
    var done: Int
    var total: Int
    var current: String
}

/// Public articles chosen to exercise the reader: formulas, code, Chinese
/// text, images, a client-rendered page and a multi-chapter tutorial.
enum Seed {
    static let articles: [String] = [
        "https://en.wikipedia.org/wiki/Rendering_equation",
        "https://en.wikipedia.org/wiki/Path_tracing",
        "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html",
        "https://zh.wikipedia.org/wiki/%E5%85%89%E7%BA%BF%E8%BF%BD%E8%B8%AA",
        "https://en.wikipedia.org/wiki/Monte_Carlo_integration",
        "https://en.wikipedia.org/wiki/Bidirectional_reflectance_distribution_function",
        "https://www.joelonsoftware.com/2003/10/08/the-absolute-minimum-every-software-developer-absolutely-positively-must-know-about-unicode-and-character-sets-no-excuses/",
        "https://raytracing.github.io/books/RayTracingInOneWeekend.html",
    ]
    static let tutorial = "https://docs.python.org/3/tutorial/index.html"
    static let tags = ["渲染", "数学", "Rust", "Python", "编码"]
}

extension DemoStore {
    func seedIfNeeded() async {
        guard let engine, items(.all).isEmpty, series.isEmpty else { return }
        let total = Seed.articles.count + 2
        var saved: [String: ItemView] = [:]
        for (i, url) in Seed.articles.enumerated() {
            seed = SeedProgress(done: i, total: total, current: URL(string: url)?.host() ?? url)
            if let item = try? await engine.saveUrl(url: url) {
                saved[url] = item
            }
            reload()
        }

        seed = SeedProgress(done: Seed.articles.count, total: total, current: "Python 教程目录")
        if let proposal = try? await engine.detectSeriesAt(url: Seed.tutorial),
           let seriesID = try? engine.createSeries(title: proposal.title, tocUrl: proposal.tocUrl, chapters: proposal.chapters) {
            let chapters = seriesChapters(seriesID)
            // Read the first chapter, start the second.
            for (index, chapter) in chapters.prefix(2).enumerated() {
                if let doc = try? await engine.document(itemId: chapter.id), let block = index == 0 ? doc.blocks.last : doc.blocks.dropFirst(doc.blocks.count / 3).first {
                    _ = try? engine.recordProgress(itemId: chapter.id, blockId: block.id, offset: 0, force: true)
                }
            }
        }

        seed = SeedProgress(done: total - 1, total: total, current: "阅读进度和标签")
        var tagIDs: [String: String] = [:]
        for name in Seed.tags {
            tagIDs[name] = try? engine.createTag(name: name)
        }
        func tag(_ url: String, _ names: [String]) {
            guard let item = saved[url] else { return }
            for name in names { if let id = tagIDs[name] { try? engine.setItemTag(itemId: item.id, tagId: id, on: true) } }
        }
        tag(Seed.articles[0], ["渲染", "数学"])
        tag(Seed.articles[1], ["渲染"])
        tag(Seed.articles[2], ["Rust"])
        tag(Seed.articles[4], ["数学"])
        tag(Seed.articles[5], ["渲染", "数学"])
        tag(Seed.articles[6], ["编码"])
        tag(Seed.articles[7], ["渲染"])
        // Partly read pages populate "continue reading".
        for (url, fraction) in [(Seed.articles[0], 0.45), (Seed.articles[2], 0.3), (Seed.articles[6], 0.6)] {
            guard let item = saved[url], let doc = try? await engine.document(itemId: item.id), !doc.blocks.isEmpty else { continue }
            let block = doc.blocks[min(doc.blocks.count - 1, Int(Double(doc.blocks.count) * fraction))]
            _ = try? engine.recordProgress(itemId: item.id, blockId: block.id, offset: 0, force: true)
            try? await engine.setRead(itemId: item.id, read: true)
        }
        if let item = saved[Seed.articles[4]] { try? await engine.setRead(itemId: item.id, read: true) }
        if let item = saved[Seed.articles[1]] { try? await engine.setStarred(itemId: item.id, starred: true) }
        seed = nil
        reload()
    }

}
