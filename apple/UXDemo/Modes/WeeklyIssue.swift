import EddyKit
import SwiftUI

/// Subscriptions laid out as a magazine issue: a masthead, sections by
/// category, a lead story per section and the rest in columns. Daily or
/// weekly issues, with older issues one click away.
struct WeeklyIssue: View {
    @Environment(DemoStore.self) private var store
    let open: (ReaderMode.Reading) -> Void
    @AppStorage("weekly.cadence") private var cadence: Cadence = .weekly
    @State private var issueIndex = 0

    enum Cadence: String, CaseIterable, Identifiable {
        case daily = "日刊"
        case weekly = "周刊"
        var id: String { rawValue }
    }

    struct Issue {
        var start: Date
        var end: Date
        var items: [ItemView]
    }

    struct Section: Identifiable {
        var id: String { name }
        var name: String
        var lead: ItemView
        var rest: [ItemView]
    }

    private var issues: [Issue] {
        let calendar = Calendar.current
        let all = store.items(.all, limit: 1000)
        let groups = Dictionary(grouping: all) { item -> Date in
            switch cadence {
            case .daily: calendar.startOfDay(for: item.date)
            case .weekly: calendar.dateInterval(of: .weekOfYear, for: item.date)?.start ?? calendar.startOfDay(for: item.date)
            }
        }
        return groups.keys.sorted(by: >).map { start in
            let end = cadence == .daily ? start : calendar.date(byAdding: .day, value: 6, to: start) ?? start
            return Issue(start: start, end: end, items: groups[start]!.sorted { $0.publishedAt > $1.publishedAt })
        }
    }

    private func sections(_ issue: Issue) -> [Section] {
        let byCategory = Dictionary(grouping: issue.items) { item -> String in
            if item.kind == .saved { return "收藏与稍后读" }
            return store.sources.first { $0.id == item.sourceId }?.category ?? "订阅"
        }
        return byCategory.keys.sorted { a, b in
            // Subscriptions first, saved pages last.
            (a == "收藏与稍后读" ? 1 : 0, a) < (b == "收藏与稍后读" ? 1 : 0, b)
        }.map { name in
            var items = byCategory[name]!
            let leadIndex = items.firstIndex { !$0.read } ?? 0
            let lead = items.remove(at: leadIndex)
            return Section(name: name, lead: lead, rest: items)
        }
    }

    var body: some View {
        let issues = issues
        ScrollView {
            if issues.isEmpty {
                ContentUnavailableView("还没有内容", systemImage: "newspaper", description: Text("在设置里连接 FreshRSS 后，订阅会编成每期杂志。"))
                    .padding(80)
            } else {
                let issue = issues[min(issueIndex, issues.count - 1)]
                VStack(alignment: .leading, spacing: 36) {
                    masthead(issue, count: issues.count)
                    ForEach(sections(issue)) { section in
                        SectionView(section: section) { id in
                            open(.init(itemID: id, title: issueTitle(issue), sequence: issue.items.map(\.id)))
                        }
                    }
                    if store.status?.mode == .unconfigured {
                        Text("示例数据里没有订阅源，这一期由保存的网页组成。在设置里连接 FreshRSS 后，订阅会按分类排进每一期。")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 48)
                .padding(.bottom, 48)
                .frame(maxWidth: 1180)
                .frame(maxWidth: .infinity)
            }
        }
        .onChange(of: cadence) { _, _ in issueIndex = 0 }
    }

    private func issueTitle(_ issue: Issue) -> String {
        "Eddy \(cadence.rawValue) · \(issue.start.formatted(.dateTime.month().day()))"
    }

    private func masthead(_ issue: Issue, count: Int) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                Text("Eddy \(cadence.rawValue)").font(.system(size: 44, weight: .black, design: .serif))
                Spacer()
                Picker("", selection: $cadence) { ForEach(Cadence.allCases) { Text($0.rawValue).tag($0) } }
                    .pickerStyle(.segmented)
                    .frame(width: 140)
            }
            HStack(spacing: 14) {
                Button { issueIndex += 1 } label: { Image(systemName: "chevron.left") }.disabled(issueIndex + 1 >= count).help("上一期")
                Text(rangeText(issue)).font(.headline)
                Button { issueIndex -= 1 } label: { Image(systemName: "chevron.right") }.disabled(issueIndex == 0).help("下一期")
                Spacer()
                let minutes = issue.items.reduce(0) { $0 + $1.readingMinutes }
                let unread = issue.items.filter { !$0.read }.count
                Text("\(issue.items.count) 篇 · 约 \(minutes) 分钟 · \(unread) 篇未读").foregroundStyle(.secondary)
            }
            .buttonStyle(.borderless)
            Rectangle().fill(Color.primary).frame(height: 2)
        }
    }

    private func rangeText(_ issue: Issue) -> String {
        let week = Calendar.current.component(.weekOfYear, from: issue.start)
        if cadence == .daily { return issue.start.formatted(.dateTime.year().month().day().weekday()) }
        return "第 \(week) 期 · \(issue.start.formatted(.dateTime.month().day())) – \(issue.end.formatted(.dateTime.month().day()))"
    }
}

private struct SectionView: View {
    let section: WeeklyIssue.Section
    let open: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(section.name.uppercased())
                .font(.system(size: 13, weight: .heavy))
                .kerning(1.5)
                .foregroundStyle(Color.accentColor)
            LeadStory(item: section.lead).onTapGesture { open(section.lead.id) }
            if !section.rest.isEmpty {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 280), spacing: 24, alignment: .top)], alignment: .leading, spacing: 22) {
                    ForEach(section.rest) { item in
                        StoryBrief(item: item).onTapGesture { open(item.id) }
                    }
                }
            }
            Divider()
        }
    }
}

private struct LeadStory: View {
    @Environment(DemoStore.self) private var store
    let item: ItemView

    var body: some View {
        HStack(alignment: .top, spacing: 24) {
            Cover(item: item, titleSize: 22)
                .frame(width: 460, height: 280)
                .clipShape(RoundedRectangle(cornerRadius: 6))
            VStack(alignment: .leading, spacing: 12) {
                Text(item.title)
                    .font(.system(size: 30, weight: .bold, design: .serif))
                    .lineLimit(4)
                    .foregroundStyle(item.read ? .secondary : .primary)
                Text(store.cachedSummary(item.id)?.tldr ?? item.excerpt ?? "")
                    .font(.system(size: 16, design: .serif))
                    .foregroundStyle(.secondary)
                    .lineLimit(6)
                Spacer(minLength: 0)
                Byline(item: item)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(minHeight: 280)
        .contentShape(Rectangle())
    }
}

private struct StoryBrief: View {
    @Environment(DemoStore.self) private var store
    let item: ItemView

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 6) {
                Text(item.title)
                    .font(.system(size: 17, weight: .semibold, design: .serif))
                    .lineLimit(3)
                    .foregroundStyle(item.read ? .secondary : .primary)
                Text(store.cachedSummary(item.id)?.tldr ?? item.excerpt ?? "")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
                Byline(item: item)
            }
            Spacer(minLength: 0)
            Cover(item: item, titleSize: 9).frame(width: 84, height: 84).clipShape(RoundedRectangle(cornerRadius: 4))
        }
        .contentShape(Rectangle())
    }
}

private struct Byline: View {
    let item: ItemView

    var body: some View {
        HStack(spacing: 6) {
            Text(item.sourceName)
            Text("·")
            Text("\(item.readingMinutes) 分钟")
            if item.progress > 0.02 && !item.finished { ProgressRing(value: item.progress, size: 11) }
            if item.starred { Image(systemName: "star.fill").foregroundStyle(.yellow) }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }
}
