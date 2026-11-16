import Foundation
import SwiftUI

/// Plain system controls for the remaining upstream form fields. Shapes and
/// defaults follow react-ui 0.18.1; drawing, entry and form validation share
/// the same value reader. Date objects cross JSON as ISO-8601 UTC strings.
/// Options have no view-level count cap; runtime data limits bound them, and
/// lazy viewports realize visible rows. Only slider positions/thumbs and the
/// TextArea viewport have fixed presentation bounds.
struct SystemFieldView: View {
    struct Choice {
        var value: String
        var label: String
        var disabled: Bool
        var checked: Bool
    }

    let element: Element
    let name: String
    let rules: [ValidationRule]
    let choices: [Choice]
    let lower: Double
    let upper: Double
    let step: Double?
    let rows: Double
    // Fixed platform-work bounds, independent of a program's numeric range.
    static let maximumPositions = 1_000
    static let maximumThumbs = 16
    @Environment(\.liveProgram) private var live
    @Environment(\.formName) private var formName
    @Environment(\.formContext) private var form
    @FocusState private var focused: Bool
    @Environment(\.calendar) private var calendar
    @Environment(\.timeZone) private var timeZone

    init(_ props: PropReader) throws {
        element = props.element
        name = try props.text("name").plain
        rules = Validation.rules(props.value("rules"))
        let part: String? = switch element.typeName {
        case "RadioGroup": "RadioItem"
        case "CheckBoxGroup": "CheckBoxItem"
        case "Chips": "ChipItem"
        case "OptionCards": "OptionCard"
        default: nil
        }
        choices = try part.map { part in
            try (props.optionalList("items") ?? []).compactMap { value -> Choice? in
                if case .scalar(.null) = value { return nil }
                guard case let .element(item) = value, item.typeName == part else { throw PropReader.Unusable() }
                let p = PropReader(element: item)
                let value = try p.text(part == "CheckBoxItem" ? "name" : "value").plain
                if (part == "ChipItem" || part == "OptionCard"), value.isEmpty { return nil }
                return Choice(value: value, label: try p.text(part == "OptionCard" ? "title" : "label").plain,
                              disabled: p.bool("disabled"), checked: p.bool("defaultChecked"))
            }
        } ?? []
        lower = props.value("min")?.json.number ?? 0
        upper = props.value("max")?.json.number ?? 100
        let increment = props.value("step")?.json.number
        let effectiveStep = props.keyword("variant", "continuous") == "discrete" ? (increment ?? 1) : increment
        if element.typeName == "Slider" {
            // SwiftUI eagerly allocates step positions. Never derive a view
            // count from the range; large grids use a tickless track instead.
            let span = upper - lower
            guard lower.isFinite, upper.isFinite, lower < upper,
                  span.isFinite, (1 / span).isFinite
            else { throw PropReader.Unusable() }
            if let effectiveStep {
                guard effectiveStep.isFinite, effectiveStep > 0 else { throw PropReader.Unusable() }
            }
        }
        step = effectiveStep
        let requestedRows = props.value("rows")?.json.number ?? 3
        rows = requestedRows.isFinite ? min(max(requestedRows, 1), 20) : 3
    }

    private var props: PropReader { PropReader(element: element) }
    private var value: JSONValue? { FieldValue(props, name: name).value(in: live, form: formName, includingCompositeDefaults: true) }
    private var selection: [String] { SelectionField.selection(props, stored: live?.field(form: formName, name: name)) }
    private var multiple: Bool { props.keyword("type", element.typeName == "Chips" ? "multiple" : "single") != "single" }

    var body: some View {
        control
            .disabled(!(live?.acceptsInteractions ?? true))
            .onAppear { hydrate() }
            .onChange(of: live?.isStreaming) { hydrate() }
    }

    @ViewBuilder private var control: some View {
        switch element.typeName {
        case "TextArea":
            TextEditor(text: Binding(get: { value?.string ?? "" }, set: { write(.string($0), clear: true) }))
                .frame(minHeight: rows * 20, idealHeight: rows * 20)
                .overlay(alignment: .topLeading) {
                    if value?.string?.isEmpty != false {
                        Text(props.value("placeholder")?.json.string ?? "").allowsHitTesting(false)
                    }
                }
                .accessibilityLabel(props.value("placeholder")?.json.string ?? name)
                .focused($focused)
                .onChange(of: focused) {
                    if focused { form?.validation.clear(name) }
                    else { validate() }
                }
        case "RadioGroup":
            choiceScroll {
                ForEach(choices.indices, id: \.self) { i in
                    let item = choices[i]
                    Picker(name, selection: Binding(get: { (value == .null ? nil : value)?.string ?? props.value("defaultValue")?.json.string ?? "" }, set: { write(.string($0)) })) {
                        Text(item.label).tag(item.value)
                    }
                    .pickerStyle(.radioGroup)
                    .labelsHidden()
                }
            }
        case "CheckBoxGroup":
            let checkedValues = Dictionary(checks.map { ($0.key, $0.value) }, uniquingKeysWith: { _, last in last })
            choiceScroll {
                ForEach(choices.indices, id: \.self) { i in
                    let item = choices[i]
                    Toggle(item.label, isOn: Binding(get: { checkedValues[item.value] ?? false }, set: { checked in
                        let entries = checks.map {
                            ($0.key, JSONValue.bool($0.key == item.value ? checked : $0.value))
                        }
                        write(.object(entries))
                    }))
                    .toggleStyle(.checkbox)
                }
            }
        case "Chips":
            ScrollView(.horizontal) {
                LazyHStack(alignment: .top) { selectionChoices.toggleStyle(.button) }
            }
            .frame(height: 44)
        case "OptionCards":
            choiceScroll { selectionChoices.toggleStyle(.checkbox) }
        case "Slider":
            if let numbers = sliderNumbers {
                VStack(alignment: .leading) {
                    Text(props.value("label")?.json.string ?? name)
                    ForEach(numbers.indices, id: \.self) { index in
                        let number = numbers[index]
                        let binding = Binding(get: { min(max(number, lower), upper) }, set: { picked in
                            guard picked.isFinite else { return }
                            var next = numbers
                            next[index] = snapped(picked)
                            write(.array(next.sorted().map(JSONValue.number)))
                        })
                        // Continuous sliders never receive SwiftUI's step.
                        // Count includes both endpoints; only bounded grids
                        // reach SwiftUI's position builder.
                        if props.keyword("variant", "continuous") == "discrete",
                           let step, step <= upper - lower,
                           (upper - lower) / step <= Double(Self.maximumPositions - 1) {
                            Slider(value: binding, in: lower...upper, step: step)
                        } else {
                            Slider(value: binding, in: lower...upper)
                        }
                    }
                }
            } else {
                // Do not truncate, sort or rewrite oversized stored values.
                Text("Slider: unsupported values (read only)")
            }
        case "DatePicker":
            if props.keyword("mode", "single") == "range" {
                VStack(alignment: .leading) {
                    datePicker("From", key: "from")
                    datePicker("To", key: "to")
                }
            } else { datePicker(name, key: nil) }
        default: EmptyView()
        }
    }

    /// Inspect raw counts before filtering or allocating thumb values.
    /// Malformed entries cannot shrink an oversized array past the bound.
    private var sliderNumbers: [Double]? {
        let current = value
        let stored = (current == .null ? nil : current)?.array
        let numbers: [Double]
        if let stored {
            guard stored.count <= Self.maximumThumbs else { return nil }
            numbers = stored.compactMap(\.number)
        } else if case let .list(items)? = props.value("defaultValue") {
            guard items.count <= Self.maximumThumbs else { return nil }
            numbers = items.compactMap { $0.json.number }
        } else {
            numbers = props.value("defaultValue")?.json.number.map { [$0] } ?? []
        }
        guard numbers.allSatisfy(\.isFinite) else { return nil }
        return numbers.isEmpty ? [lower] : numbers
    }

    /// Bounded viewport: lazy rows are instantiated only as they become visible.
    private func choiceScroll<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        ScrollView {
            LazyVStack(alignment: .leading) { content() }
        }
        .frame(height: CGFloat(min(choices.count, 8)) * 30)
    }

    private var dayCalendar: Calendar {
        var result = calendar
        result.timeZone = timeZone
        return result
    }

    /// Round to the nearest min-relative step, then clamp, without building a
    /// grid or converting a potentially enormous position count to an Int.
    private func snapped(_ picked: Double) -> Double {
        guard let step else { return min(max(picked, lower), upper) }
        let distance = min(max(picked, lower), upper) - lower
        // Match JavaScript Math.round((position - min) / step), including
        // positive half ties. A remainder-based rounding differs at 0.5/0.2.
        let offset = floor(distance / step + 0.5) * step
        var result = lower + offset
        // Remove decimal grid arithmetic noise (for example 0.1 + 0.2).
        // Decimal/exponent parsing is bounded by Double's printed width.
        let parts = String(step).lowercased().split(separator: "e")
        let mantissa = parts[0].split(separator: ".", omittingEmptySubsequences: false)
        let fraction = mantissa.count == 2 ? mantissa[1].count : 0
        let exponent = parts.count == 2 ? Int(parts[1]) ?? 0 : 0
        let places = min(max(fraction - exponent, 0), 324)
        let scale = pow(10, Double(places))
        if scale.isFinite, (result * scale).isFinite {
            result = floor(result * scale + 0.5) / scale
        }
        return min(max(result, lower), upper)
    }

    /// Upstream builds one name-keyed aggregate before drawing any item.
    /// Duplicate item names share the last default and one stored boolean.
    private var checks: [(key: String, value: Bool)] {
        let stored: [String: Bool]
        if case let .object(entries)? = value {
            stored = Dictionary(entries.compactMap { entry in entry.value.bool.map { (entry.key, $0) } }, uniquingKeysWith: { _, last in last })
        } else { stored = [:] }
        var values: [String: Bool] = [:]
        var names: [String] = []
        for item in choices {
            if values.updateValue(stored[item.value] ?? item.checked, forKey: item.value) == nil {
                names.append(item.value)
            }
        }
        return names.map { ($0, values[$0] ?? false) }
    }

    private var selectionChoices: some View {
        let selectedValues = Set(selection)
        return ForEach(choices.indices, id: \.self) { i in
            let item = choices[i]
            Toggle(item.label, isOn: Binding(get: { selectedValues.contains(item.value) }, set: { selected in
                let held = selection
                guard selected != held.contains(item.value) else { return }
                let next = multiple ? (selected ? held + [item.value] : held.filter { $0 != item.value }) : (selected ? [item.value] : [])
                write(multiple ? .array(next.map(JSONValue.string)) : next.first.map(JSONValue.string))
            }))
            .disabled(item.disabled)
        }
    }

    private func datePicker(_ label: String, key: String?) -> some View {
        let current = key.flatMap { value?[$0] } ?? (key == nil ? value : nil)
        return DatePicker(label, selection: Binding(get: { Self.date(current) ?? Date(timeIntervalSince1970: 0) }, set: { date in
            let stored = JSONValue.string(Self.dateText(dayCalendar.startOfDay(for: date)))
            if let key {
                let entries: [(String, JSONValue)] = ["from", "to"].compactMap { k in
                    k == key ? (k, stored) : value?[k].map { (k, $0) }
                }
                write(.object(entries))
            } else { write(stored) }
        }), displayedComponents: .date)
        .datePickerStyle(.field)
    }

    private static func date(_ value: JSONValue?) -> Date? {
        guard let text = value?.string else { return nil }
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = formatter.date(from: text) { return date }
        formatter.formatOptions = [.withInternetDateTime]
        if let date = formatter.date(from: text) { return date }
        formatter.formatOptions = [.withFullDate]
        return formatter.date(from: text)
    }

    private static func dateText(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.string(from: date)
    }

    private func write(_ value: JSONValue?, clear: Bool = false) {
        guard let live else { return }
        live.attempt { try live.enter(value, into: element, form: formName) }
        guard live.failure == nil else { return }
        if clear { form?.validation.clear(name) } else { validate() }
    }

    private func validate() {
        guard let live else { return }
        form?.validation.validate(name, value: SelectionField.isSelection(element) ? SelectionField.validationValue(props, live: live, form: formName, name: name) : value, rules: rules)
    }

    private func hydrate() {
        guard SelectionField.isSelection(element), let live, !live.isStreaming else { return }
        do { try live.initializeSelection(element, form: formName) }
        catch { live.attempt { throw error } }
    }
}

/// Selection fields have distinct display and validation normalization upstream.
/// A wrong-shaped stored value may still draw a selection but fail required.
@MainActor enum SelectionField {
    static func isSelection(_ element: Element) -> Bool { element.typeName == "Chips" || element.typeName == "OptionCards" }
    static func single(_ props: PropReader) -> Bool { props.keyword("type", props.element.typeName == "Chips" ? "multiple" : "single") == "single" }

    static func selection(_ props: PropReader, stored: JSONValue?) -> [String] {
        let fallback = props.value("defaultValue")?.json
        for candidate in [stored, fallback] {
            if single(props) {
                if let text = candidate?.string ?? candidate?.array?.first?.string { return text.isEmpty ? [] : [text] }
            } else {
                if let array = candidate?.array { return array.compactMap(\.string) }
                if let text = candidate?.string { return text.isEmpty ? [] : [text] }
            }
        }
        return []
    }

    static func defaultValue(_ props: PropReader) -> JSONValue? {
        guard let value = props.value("defaultValue")?.json else { return nil }
        let items = selection(props, stored: value)
        if single(props) { return items.first.map(JSONValue.string) }
        guard value.string != nil || value.array != nil else { return nil }
        return .array(items.map(JSONValue.string))
    }

    static func validationValue(_ props: PropReader, live: LiveProgram, form: String?, name: String) -> JSONValue? {
        let stored = live.field(form: form, name: name)
        let value = stored == .null ? defaultValue(props) : stored ?? defaultValue(props)
        if single(props) { return value?.string.flatMap { $0.isEmpty ? nil : .string($0) } }
        return value?.array.flatMap { $0.isEmpty ? nil : .array($0) }
    }
}
