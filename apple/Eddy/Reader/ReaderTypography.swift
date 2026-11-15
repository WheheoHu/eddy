import AppKit
import EddyKit
import SwiftUI

/// The reader's typefaces: one for Latin text, one for Chinese. Each is
/// `systemSans`, `systemSerif` or the family name of an installed font.
/// Stored on this Mac only, in user defaults.
struct ReaderFonts: Equatable {
    static let systemSans = "system-sans"
    static let systemSerif = "system-serif"
    static let latinKey = "reader.latinFont"
    static let cjkKey = "reader.cjkFont"
    static let spacingKey = "reader.cjkLatinSpacing"
    static let justifyKey = "reader.justify"

    var latin = ReaderFonts.systemSans
    var cjk = ReaderFonts.systemSans
    /// A quarter em between Chinese and Latin text that has no space.
    var spacing = true

    /// What the reading settings hold now, for work outside a view.
    static var stored: ReaderFonts {
        let d = UserDefaults.standard
        return ReaderFonts(
            latin: d.string(forKey: latinKey) ?? systemSans,
            cjk: d.string(forKey: cjkKey) ?? systemSans,
            spacing: d.object(forKey: spacingKey) as? Bool ?? true
        )
    }

    /// The serif switch became the two font choices.
    static func migrate(_ defaults: UserDefaults = .standard) {
        guard defaults.object(forKey: "reader.serif") != nil else { return }
        if defaults.bool(forKey: "reader.serif") {
            if defaults.string(forKey: latinKey) == nil { defaults.set(systemSerif, forKey: latinKey) }
            if defaults.string(forKey: cjkKey) == nil { defaults.set(systemSerif, forKey: cjkKey) }
        }
        defaults.removeObject(forKey: "reader.serif")
    }

    // MARK: Fonts

    func latinFont(size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
        let system = NSFont.systemFont(ofSize: size, weight: weight)
        switch latin {
        case Self.systemSans:
            return system
        case Self.systemSerif:
            return system.fontDescriptor.withDesign(.serif).flatMap { NSFont(descriptor: $0, size: size) } ?? system
        default:
            return Self.font(family: latin, size: size, weight: weight) ?? system
        }
    }

    /// Named outright rather than left to the system's fallback: next to a
    /// system font, a fallback Songti sets ，。： a half em too wide.
    func cjkFont(size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
        let family = switch cjk {
        case Self.systemSans: "PingFang SC"
        case Self.systemSerif: "Songti SC"
        default: cjk
        }
        return Self.font(family: family, size: size, weight: weight)
            ?? Self.font(family: "PingFang SC", size: size, weight: weight)
            ?? .systemFont(ofSize: size, weight: weight)
    }

    /// Nil when the family isn't installed.
    private static func font(family: String, size: CGFloat, weight: NSFont.Weight) -> NSFont? {
        let named = NSFontDescriptor(fontAttributes: [.family: family])
        guard named.matchingFontDescriptor(withMandatoryKeys: [.family]) != nil else { return nil }
        let weighted = named.addingAttributes([.traits: [NSFontDescriptor.TraitKey.weight: weight.rawValue]])
        return NSFont(descriptor: weighted, size: size)
    }

    static func isInstalled(_ choice: String) -> Bool {
        choice == systemSans || choice == systemSerif || font(family: choice, size: 12, weight: .regular) != nil
    }

    /// Installed families, for the Latin picker.
    static let latinFamilies: [String] = NSFontManager.shared.availableFontFamilies
        .filter { !$0.hasPrefix(".") }
        .sorted { $0.localizedStandardCompare($1) == .orderedAscending }

    /// Installed families that have Chinese characters.
    static let cjkFamilies: [String] = {
        let wanted = NSFontDescriptor(fontAttributes: [.characterSet: CharacterSet(charactersIn: "的一是")])
        let families = wanted.matchingFontDescriptors(withMandatoryKeys: [.characterSet])
            .compactMap { $0.object(forKey: .family) as? String }
            .filter { !$0.hasPrefix(".") }
        return Set(families).sorted { $0.localizedStandardCompare($1) == .orderedAscending }
    }()

    // MARK: Typesetting

    /// Sets the Chinese in `text` in the Chinese font, closes up adjacent
    /// full-width marks and, if wanted, opens Chinese/Latin boundaries.
    /// Code keeps its own font and spacing.
    func typeset(_ text: NSMutableAttributedString) {
        let string = text.string
        guard string.utf16.contains(where: { $0 >= 0x2E80 && $0 != 0xFFFC }) else { return }
        var fonts: [String: NSFont] = [:]
        for span in typographySpans(text: string) {
            let range = NSRange(location: Int(span.start), length: Int(span.end - span.start))
            guard NSMaxRange(range) <= text.length else { continue }
            switch span.kind {
            case .cjk:
                var changes: [(NSRange, NSFont)] = []
                text.enumerateAttribute(.font, in: range) { value, sub, _ in
                    guard let font = value as? NSFont, !Self.isCode(font) else { return }
                    let weight = Self.weight(of: font)
                    let key = "\(font.pointSize)|\(weight.rawValue)"
                    let cjk = fonts[key] ?? cjkFont(size: font.pointSize, weight: weight)
                    fonts[key] = cjk
                    changes.append((sub, cjk))
                }
                for (sub, font) in changes { text.addAttribute(.font, value: font, range: sub) }
            case .squeeze, .gap:
                if span.kind == .gap, !spacing { continue }
                guard let font = text.attribute(.font, at: range.location, effectiveRange: nil) as? NSFont, !Self.isCode(font) else { continue }
                text.addAttribute(.kern, value: font.pointSize * (span.kind == .squeeze ? -0.5 : 0.25), range: range)
            }
        }
    }

    private static func isCode(_ font: NSFont) -> Bool {
        font.fontDescriptor.symbolicTraits.contains(.monoSpace)
    }

    private static func weight(of font: NSFont) -> NSFont.Weight {
        let traits = font.fontDescriptor.object(forKey: .traits) as? [NSFontDescriptor.TraitKey: Any]
        if let raw = traits?[.weight] as? CGFloat { return NSFont.Weight(raw) }
        return font.fontDescriptor.symbolicTraits.contains(.bold) ? .bold : .regular
    }
}

/// A menu of the system's two faces and the installed families. A chosen
/// font that has since been removed stays listed, marked as missing.
struct ReaderFontPicker: View {
    let title: LocalizedStringKey
    @Binding var selection: String
    let families: [String]

    var body: some View {
        Picker(title, selection: $selection) {
            Text("System Sans-Serif").tag(ReaderFonts.systemSans)
            Text("System Serif").tag(ReaderFonts.systemSerif)
            Divider()
            if !ReaderFonts.isInstalled(selection) {
                Text("\(selection) (Not Installed)").tag(selection)
            }
            ForEach(families, id: \.self) { Text(verbatim: $0).tag($0) }
        }
    }
}
