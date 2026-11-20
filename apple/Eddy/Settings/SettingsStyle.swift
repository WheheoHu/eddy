import SwiftUI

/// A settings page: sections with a bold heading over rows ruled off from
/// each other, and any note under them. Pages are written as an ordinary
/// `Form`; this lays it out in the app's own language in place of the
/// system's grouped cards.
struct PanelFormStyle: FormStyle {
    func makeBody(configuration: Configuration) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                ForEach(sections: configuration.content) { section in
                    VStack(alignment: .leading, spacing: 0) {
                        if !section.header.isEmpty {
                            section.header
                                .font(Theme.heading)
                                .foregroundStyle(Theme.text1)
                                .padding(.bottom, 8)
                        }
                        Hairline()
                        ForEach(subviews: section.content) { row in
                            row
                                .frame(maxWidth: .infinity, minHeight: 30, alignment: .leading)
                                .padding(.vertical, 7)
                            Hairline()
                        }
                        if !section.footer.isEmpty {
                            section.footer
                                .font(.system(size: 12))
                                .foregroundStyle(Theme.text2)
                                .lineSpacing(2)
                                .frame(maxWidth: 560, alignment: .leading)
                                .fixedSize(horizontal: false, vertical: true)
                                .padding(.top, 8)
                        }
                    }
                }
            }
            .padding(.horizontal, 28)
            .padding(.top, 4)
            .padding(.bottom, 32)
        }
    }
}

/// A name on the left and its value or control on the right; the control
/// drops under the name when the page is too narrow for both.
struct PanelLabeledStyle: LabeledContentStyle {
    func makeBody(configuration: Configuration) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 16) {
                configuration.label.fontWeight(.medium).foregroundStyle(Theme.text1)
                Spacer(minLength: 0)
                configuration.content.foregroundStyle(Theme.text2)
            }
            VStack(alignment: .leading, spacing: 6) {
                configuration.label.fontWeight(.medium).foregroundStyle(Theme.text1)
                configuration.content.foregroundStyle(Theme.text2)
            }
        }
    }
}

extension View {
    /// The styles every settings page is drawn with.
    func settingsPageStyle() -> some View {
        formStyle(PanelFormStyle())
            .labeledContentStyle(PanelLabeledStyle())
            .toggleStyle(FlatToggleStyle())
            .buttonStyle(KeyButtonStyle())
            .font(.system(size: 13.5))
            .foregroundStyle(Theme.text1)
    }
}

/// A name with words under it, for a row whose control needs explaining.
private struct SettingsLabel: View {
    let title: LocalizedStringKey
    let note: LocalizedStringKey?

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(title)
            if let note {
                Text(note).font(.system(size: 12)).fontWeight(.regular).foregroundStyle(Theme.text2)
            }
        }
    }
}

/// A place to type, named on the left.
struct SettingsField: View {
    let title: LocalizedStringKey
    @Binding var text: String
    var prompt: Text?
    var note: LocalizedStringKey?
    var secure = false
    /// More than one line of text, growing between these.
    var lines: ClosedRange<Int>?
    var width: CGFloat = 260
    @FocusState private var focused: Bool

    init(_ title: LocalizedStringKey, text: Binding<String>, prompt: Text? = nil, note: LocalizedStringKey? = nil, secure: Bool = false, lines: ClosedRange<Int>? = nil, width: CGFloat = 260) {
        self.title = title
        _text = text
        self.prompt = prompt
        self.note = note
        self.secure = secure
        self.lines = lines
        self.width = width
    }

    var body: some View {
        LabeledContent {
            field
                .flatField(focused: focused)
                .focused($focused)
                .foregroundStyle(Theme.text1)
                .frame(width: width)
                .accessibilityLabel(Text(title))
        } label: {
            SettingsLabel(title: title, note: note)
        }
    }

    @ViewBuilder private var field: some View {
        if secure {
            SecureField("", text: $text, prompt: prompt)
        } else if let lines {
            TextField("", text: $text, prompt: prompt, axis: .vertical).lineLimit(lines)
        } else {
            TextField("", text: $text, prompt: prompt)
        }
    }
}

/// One of a few, as a switch.
struct SettingsChoice<Value: Hashable>: View {
    let title: LocalizedStringKey
    @Binding var selection: Value
    let options: [(Value, String)]

    init(_ title: LocalizedStringKey, selection: Binding<Value>, options: [(Value, String)]) {
        self.title = title
        _selection = selection
        self.options = options
    }

    var body: some View {
        LabeledContent {
            SwitchBar(selection: $selection, options: options.map { .text($0.0, $0.1) })
        } label: {
            Text(title)
        }
    }
}

/// One of many, as a pop-up menu.
struct SettingsPicker<Value: Hashable, Content: View>: View {
    let title: LocalizedStringKey
    @Binding var selection: Value
    @ViewBuilder let content: Content

    init(_ title: LocalizedStringKey, selection: Binding<Value>, @ViewBuilder content: () -> Content) {
        self.title = title
        _selection = selection
        self.content = content()
    }

    var body: some View {
        LabeledContent {
            Picker(title, selection: $selection) { content }.labelsHidden().flatPopup()
        } label: {
            Text(title)
        }
    }
}

/// The language an AI feature writes in: Chinese or English for now. What
/// is kept is the name the model is told, as before, so a language typed in
/// when this was a text field stays listed and chosen until another is.
struct SettingsLanguage: View {
    let title: LocalizedStringKey
    @Binding var language: String
    /// What choosing none means, on pages where a language may be left to
    /// another setting.
    var unset: LocalizedStringKey?

    private static let offered: [(name: String, label: LocalizedStringKey)] = [
        ("简体中文", "Chinese (Simplified)"),
        ("English", "English"),
    ]

    init(_ title: LocalizedStringKey, language: Binding<String>, unset: LocalizedStringKey? = nil) {
        self.title = title
        _language = language
        self.unset = unset
    }

    var body: some View {
        SettingsPicker(title, selection: $language) {
            if let unset { Text(unset).tag("") }
            ForEach(Self.offered, id: \.name) { Text($0.label).tag($0.name) }
            if !language.isEmpty || unset == nil, !Self.offered.contains(where: { $0.name == language }) {
                Text(verbatim: language).tag(language)
            }
        }
    }
}

/// An amount on a groove, with its value beside it.
struct SettingsSlider: View {
    let title: LocalizedStringKey
    @Binding var value: Double
    let range: ClosedRange<Double>
    let step: Double
    let text: (Double) -> String

    init(_ title: LocalizedStringKey, value: Binding<Double>, in range: ClosedRange<Double>, step: Double, text: @escaping (Double) -> String) {
        self.title = title
        _value = value
        self.range = range
        self.step = step
        self.text = text
    }

    var body: some View {
        LabeledContent {
            HStack(spacing: 10) {
                FlatSlider(value: $value, range: range, step: step).frame(width: 220)
                Text(text(value)).monospacedDigit().frame(width: 48, alignment: .trailing)
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel(Text(title))
        } label: {
            Text(title)
        }
    }
}

/// A count, with keys to change it.
struct SettingsStepper: View {
    let title: LocalizedStringKey
    @Binding var value: Int
    let range: ClosedRange<Int>
    var step = 1

    init(_ title: LocalizedStringKey, value: Binding<Int>, in range: ClosedRange<Int>, step: Int = 1) {
        self.title = title
        _value = value
        self.range = range
        self.step = step
    }

    var body: some View {
        LabeledContent {
            FlatStepper(value: $value, range: range, step: step).accessibilityLabel(Text(title))
        } label: {
            Text(title)
        }
    }
}

/// A fact with a small square in front of it: green when all is well.
struct StatusValue: View {
    let text: String
    let good: Bool

    var body: some View {
        HStack(spacing: 7) {
            RoundedRectangle(cornerRadius: 2).fill(good ? Theme.ok : Theme.warn).frame(width: 8, height: 8)
            Text(text).lineLimit(1)
        }
    }
}
