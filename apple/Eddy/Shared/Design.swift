import SwiftUI

/// The app's one visual language. Surfaces are flat and come in two tones:
/// a track or a chosen row is a shade darker than the column it is in, and
/// the piece that sits in a track is the lightest surface, with a hairline.
/// Nothing has gradients, inner shadows or highlights; a press is a darker
/// fill. Corners are square enough to sit with the window's frame.
enum Theme {
    // Surfaces, left to right: each column a little lighter than the last.
    static let side = Color(light: 0xECECEF, dark: 0x19191B)
    static let list = Color(light: 0xF6F6F7, dark: 0x202022)
    static let page = Color(light: 0xFFFFFF, dark: 0x1C1C1E)

    static let text1 = Color(light: 0x111113, dark: 0xF2F2F3)
    static let text2 = Color(light: 0x5F5F65, dark: 0xA6A6AC)
    static let text3 = Color(light: 0x77777D, dark: 0x7C7C83)

    /// Lines between rows and columns.
    static let line = Color(light: 0x000000, dark: 0xFFFFFF, lightAlpha: 0.10, darkAlpha: 0.10)
    /// Outlines of controls.
    static let outline = Color(light: 0x000000, dark: 0xFFFFFF, lightAlpha: 0.14, darkAlpha: 0.16)

    /// Under the pointer.
    static let hover = Color(light: 0x000000, dark: 0xFFFFFF, lightAlpha: 0.05, darkAlpha: 0.08)
    /// A track, a chosen row, a control that is on.
    static let track = Color(light: 0x000000, dark: 0xFFFFFF, lightAlpha: 0.075, darkAlpha: 0.10)
    /// While held down.
    static let press = Color(light: 0x000000, dark: 0xFFFFFF, lightAlpha: 0.14, darkAlpha: 0.20)
    /// The piece that sits in a track.
    static let knob = Color(light: 0xFFFFFF, dark: 0x5C5C62)
    static let knobEdge = Color(light: 0x000000, dark: 0x000000, lightAlpha: 0.14, darkAlpha: 0.5)

    static let accent = Color(light: 0x1F5FBF, dark: 0x7DB0FF)
    static let star = Color(light: 0xD99A00, dark: 0xF2B93B)
    static let danger = Color(light: 0xC23A2B, dark: 0xFF7B6B)
    static let ok = Color(light: 0x2F9E5B, dark: 0x4FBF7C)
    static let warn = Color(light: 0xD98A1F, dark: 0xF0A63C)

    /// Over what a panel or a column covers.
    static let scrim = Color(light: 0x000000, dark: 0x000000, lightAlpha: 0.18, darkAlpha: 0.40)
    /// Behind a column that has sunk.
    static let well = Color(light: 0xA3A5AC, dark: 0x000000)

    enum Radius {
        static let control: CGFloat = 7
        static let track: CGFloat = 8
        static let knob: CGFloat = 5.5
        static let row: CGFloat = 6
        static let panel: CGFloat = 8
    }

    /// The height of the strip of buttons at the top of a column.
    static let strip: CGFloat = 52

    static func title(_ size: CGFloat = 26) -> Font { .system(size: size, weight: .bold) }
    static let heading = Font.system(size: 15, weight: .bold)
    static let body = Font.system(size: 14)
    static let small = Font.system(size: 12, weight: .medium)
}

extension Color {
    /// A colour with a value for each appearance.
    init(light: UInt32, dark: UInt32, lightAlpha: Double = 1, darkAlpha: Double = 1) {
        self.init(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            let (hex, alpha) = isDark ? (dark, darkAlpha) : (light, lightAlpha)
            return NSColor(
                srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
                blue: CGFloat(hex & 0xFF) / 255, alpha: alpha
            )
        })
    }
}

/// A one-point line in the theme's colour.
struct Hairline: View {
    var vertical = false

    var body: some View {
        Rectangle().fill(Theme.line).frame(width: vertical ? 1 : nil, height: vertical ? nil : 1)
    }
}

// MARK: - Buttons

/// An icon button: flat, a shade under the pointer, darker while held, and
/// a shade darker for as long as what it switches is on.
struct FlatButtonStyle: ButtonStyle {
    var on = false
    var size: CGFloat = 32

    func makeBody(configuration: Configuration) -> some View {
        FlatButtonBody(configuration: configuration, on: on, size: size)
    }
}

private struct FlatButtonBody: View {
    let configuration: ButtonStyle.Configuration
    let on: Bool
    let size: CGFloat
    @State private var hovering = false
    @Environment(\.isEnabled) private var enabled

    var body: some View {
        configuration.label
            .font(.system(size: size >= 30 ? 15 : 13, weight: on ? .semibold : .regular))
            .foregroundStyle(enabled ? Theme.text1 : Theme.text3.opacity(0.6))
            .padding(.horizontal, 4)
            .frame(minWidth: size, minHeight: size)
            .background(fill, in: RoundedRectangle(cornerRadius: Theme.Radius.control, style: .continuous))
            .contentShape(Rectangle())
            .onHover { hovering = $0 }
    }

    private var fill: Color {
        guard enabled else { return .clear }
        if configuration.isPressed { return Theme.press }
        if on { return Theme.track }
        return hovering ? Theme.hover : .clear
    }
}

/// A button with words on it: an outlined key that darkens while held.
struct KeyButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        KeyButtonBody(configuration: configuration)
    }
}

private struct KeyButtonBody: View {
    let configuration: ButtonStyle.Configuration
    @State private var hovering = false
    @Environment(\.isEnabled) private var enabled

    var body: some View {
        configuration.label
            .font(.system(size: 13, weight: .medium))
            .foregroundStyle(enabled ? (configuration.role == .destructive ? Theme.danger : Theme.text1) : Theme.text3.opacity(0.7))
            .lineLimit(1)
            .padding(.horizontal, 10)
            .frame(minWidth: 28, minHeight: 28)
            .background(fill, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(Theme.outline))
            .contentShape(Rectangle())
            .onHover { hovering = $0 }
    }

    private var fill: Color {
        guard enabled else { return .clear }
        if configuration.isPressed { return Theme.press }
        return hovering ? Theme.hover : Theme.page
    }
}

// MARK: - Switches

/// One of a few: a track with a lighter piece on the chosen side. Used for
/// every either/or choice, from the app's mode to a setting.
struct SwitchBar<Value: Hashable>: View {
    struct Option: Identifiable {
        let value: Value
        var title: String?
        var symbol: String?
        var help: String?
        var id: Value { value }

        static func text(_ value: Value, _ title: String) -> Option { Option(value: value, title: title) }
        static func symbol(_ value: Value, _ symbol: String, help: String) -> Option { Option(value: value, symbol: symbol, help: help) }
    }

    @Binding var selection: Value
    let options: [Option]
    /// The width of a segment that shows a symbol.
    var segment: CGFloat = 40
    @Namespace private var space

    var body: some View {
        HStack(spacing: 0) {
            ForEach(options) { option in
                let chosen = option.value == selection
                Button {
                    withAnimation(.snappy(duration: 0.18)) { selection = option.value }
                } label: {
                    label(option, chosen: chosen)
                        .frame(height: 26)
                        .background {
                            if chosen {
                                RoundedRectangle(cornerRadius: Theme.Radius.knob, style: .continuous)
                                    .fill(Theme.knob)
                                    .overlay(RoundedRectangle(cornerRadius: Theme.Radius.knob, style: .continuous).strokeBorder(Theme.knobEdge, lineWidth: 0.5))
                                    .matchedGeometryEffect(id: "knob", in: space)
                            }
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(option.help ?? option.title ?? "")
                .accessibilityLabel(option.help ?? option.title ?? "")
                .accessibilityAddTraits(chosen ? .isSelected : [])
            }
        }
        .padding(3)
        .background(Theme.track, in: RoundedRectangle(cornerRadius: Theme.Radius.track, style: .continuous))
        .fixedSize()
    }

    @ViewBuilder private func label(_ option: Option, chosen: Bool) -> some View {
        if let symbol = option.symbol {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: chosen ? .semibold : .regular))
                .foregroundStyle(chosen ? Theme.text1 : Theme.text2)
                .frame(width: segment)
        } else {
            Text(option.title ?? "")
                .font(.system(size: 12.5, weight: chosen ? .medium : .regular))
                .foregroundStyle(chosen ? Theme.text1 : Theme.text2)
                .lineLimit(1)
                .padding(.horizontal, 11)
        }
    }
}

/// On or off: the same track and piece as `SwitchBar`, with the track in the
/// accent colour while on.
struct FlatToggleStyle: ToggleStyle {
    /// Without the space that pushes the switch to the trailing edge.
    var compact = false

    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: compact ? 8 : 16) {
            configuration.label.fontWeight(compact ? .regular : .medium)
            if !compact { Spacer(minLength: 0) }
            ZStack(alignment: configuration.isOn ? .trailing : .leading) {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(configuration.isOn ? Theme.accent : Theme.track)
                RoundedRectangle(cornerRadius: 4, style: .continuous)
                    .fill(configuration.isOn ? Color.white : Theme.knob)
                    .overlay(RoundedRectangle(cornerRadius: 4, style: .continuous).strokeBorder(Theme.knobEdge, lineWidth: 0.5))
                    .frame(width: 19, height: 18)
                    .padding(3)
            }
            .frame(width: 44, height: 24)
            .contentShape(Rectangle())
            .onTapGesture {
                withAnimation(.snappy(duration: 0.16)) { configuration.isOn.toggle() }
            }
        }
        .accessibilityRepresentation {
            Toggle(isOn: configuration.$isOn) { configuration.label }
        }
    }
}

/// A groove with a piece to drag along it.
struct FlatSlider: View {
    @Binding var value: Double
    let range: ClosedRange<Double>
    var step: Double = 1

    private static let knob = CGSize(width: 14, height: 20)

    var body: some View {
        GeometryReader { proxy in
            let travel = max(1, proxy.size.width - Self.knob.width)
            let fraction = (value - range.lowerBound) / (range.upperBound - range.lowerBound)
            ZStack(alignment: .leading) {
                Capsule().fill(Theme.track).frame(height: 4)
                RoundedRectangle(cornerRadius: 3.5, style: .continuous)
                    .fill(Theme.knob)
                    .overlay(RoundedRectangle(cornerRadius: 3.5, style: .continuous).strokeBorder(Theme.knobEdge, lineWidth: 0.5))
                    .shadow(color: .black.opacity(0.08), radius: 0.75, y: 1)
                    .frame(width: Self.knob.width, height: Self.knob.height)
                    .offset(x: travel * min(1, max(0, fraction)))
            }
            .frame(maxHeight: .infinity)
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { drag in
                let at = min(1, max(0, (drag.location.x - Self.knob.width / 2) / travel))
                let raw = range.lowerBound + at * (range.upperBound - range.lowerBound)
                let stepped = (raw / step).rounded() * step
                let next = min(range.upperBound, max(range.lowerBound, stepped))
                if next != value { value = next }
            })
        }
        .frame(height: 22)
        .accessibilityRepresentation { Slider(value: $value, in: range, step: step) }
    }
}

/// A number with a key on each side of a line to make it smaller or larger.
struct FlatStepper: View {
    @Binding var value: Int
    let range: ClosedRange<Int>
    var step = 1

    var body: some View {
        HStack(spacing: 8) {
            Text("\(value)").font(.system(size: 13, weight: .medium)).monospacedDigit().foregroundStyle(Theme.text1)
            HStack(spacing: 0) {
                key("minus", "Fewer", enabled: value > range.lowerBound) { value = max(range.lowerBound, value - step) }
                Rectangle().fill(Theme.outline).frame(width: 1, height: 26)
                key("plus", "More", enabled: value < range.upperBound) { value = min(range.upperBound, value + step) }
            }
            .background(Theme.page, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(Theme.outline))
            .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
        }
        .accessibilityRepresentation { Stepper("\(value)", value: $value, in: range, step: step) }
    }

    private func key(_ symbol: String, _ help: String, enabled: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol).font(.system(size: 11, weight: .medium)).frame(width: 28, height: 26).contentShape(Rectangle())
        }
        .buttonStyle(StepKeyStyle())
        .disabled(!enabled)
        .help(help)
    }
}

private struct StepKeyStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        StepKeyBody(configuration: configuration)
    }
}

private struct StepKeyBody: View {
    let configuration: ButtonStyle.Configuration
    @State private var hovering = false
    @Environment(\.isEnabled) private var enabled

    var body: some View {
        configuration.label
            .foregroundStyle(enabled ? Theme.text1 : Theme.text3.opacity(0.6))
            .background(configuration.isPressed ? Theme.press : hovering && enabled ? Theme.hover : Color.clear)
            .onHover { hovering = $0 }
    }
}

extension View {
    /// A place to type: a flat block a shade darker than the page.
    func flatField(focused: Bool = false) -> some View {
        textFieldStyle(.plain)
            .font(.system(size: 13))
            .padding(.horizontal, 9)
            .padding(.vertical, 6)
            .frame(minHeight: 28)
            .background(Theme.track, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(Theme.accent.opacity(focused ? 0.7 : 0), lineWidth: 2))
    }

    /// A pop-up menu drawn as an outlined key.
    func flatPopup() -> some View {
        pickerStyle(.menu)
            .buttonStyle(.borderless)
            .tint(Theme.text1)
            .fixedSize()
            .padding(.leading, 8)
            .padding(.trailing, 4)
            .frame(minHeight: 28)
            .background(Theme.page, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(Theme.outline))
    }
}
