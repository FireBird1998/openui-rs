import AppKit
import Foundation
import OpenUINative
import SwiftUI
import Testing

@MainActor private func fieldLibrary() async throws -> (Core, Library, String) {
    var directory = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { directory.deleteLastPathComponent() }
    let core = Core()
    let library = try await core.library(schema: String(contentsOf: directory.appendingPathComponent("conformance/imported/schemas/react-ui.json"), encoding: .utf8))
    let spec = try String(contentsOf: directory.appendingPathComponent("conformance/prompts/react-ui-schema.json"), encoding: .utf8)
    return (core, library, spec)
}

extension NativeUITests {
/// Isolated from ordinary field tests: removing a platform allocation bound
/// can abort the runner here, which the discovery/completion gate must reject.
@Suite @MainActor struct BoundedNativeControls {
    @Test(arguments: ["discrete", "continuous"])
    func enormousSliderGridsDrawWithoutChangingStateAndStillWriteArrays(_ variant: String) async throws {
        let (core, library, spec) = try await fieldLibrary()
        for (upper, step) in [(1e15, 1.0), (1.0, 1e-9), (10.0, 0.25), (10.0, 0.1), (1.0, 2.0)] {
            let live = try LiveProgram(library: library, promptSpec: spec)
            try live.show(try await core.parseOneShot("root = Slider(\"x\", \"\(variant)\", 0, \(upper), \(step), [0])", library: library))
            let state = live.state
            let version = live.version
            let window = await Window(live)
            defer { window.close() }
            #expect(live.state == state && live.version == version)
            try await window.click("/", at: { CGPoint(x: $0.minX + $0.width * 0.73, y: $0.maxY - 8) })
            let numbers = try #require(live.field(form: nil, name: "x")?.array)
            #expect(numbers.count == 1)
            let number = try #require(numbers.first?.number)
            #expect(number.isFinite && number >= 0 && number <= upper)
            if step > upper { #expect(number == 0) } else { #expect(number > 0) }
            if step == 1 { #expect(number == number.rounded()) }
            if step == 0.25 { #expect(number * 4 == (number * 4).rounded()) }
            if step == 0.1 { #expect(abs(number * 10 - (number * 10).rounded()) < 1e-10) }
        }
    }

    @Test(arguments: ["discrete", "continuous"])
    func aThousandSliderValuesStayReadOnlyUntilReplacedWithABoundedValue(_ variant: String) async throws {
        let (core, library, spec) = try await fieldLibrary()
        let values = JSONValue.array((0..<1_000).map { .number(Double($0)) })
        let live = try LiveProgram(library: library, promptSpec: spec)
        try live.show(try await core.parseOneShot("root = Slider(\"x\", \"\(variant)\", 0, 1000, 1, \(values.compact))", library: library))
        for stored in [nil, values, .array(Array(repeating: JSONValue.null, count: 1_000))] {
            if let stored { try live.setField(form: nil, name: "x", to: stored) }
            let state = live.state
            let version = live.version
            let window = await Window(live)
            defer { window.close() }
            #expect(live.state == state && live.version == version)
            try await window.click("/", at: { CGPoint(x: $0.midX, y: $0.maxY - 8) })
            #expect(live.state == state && live.version == version)
        }
        // The read-only oversized value is never truncated by drawing. The
        // app can replace it, after which actual user interaction works again.
        try live.setField(form: nil, name: "x", to: .array([.number(1)]))
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/", at: { CGPoint(x: $0.minX + $0.width * 0.73, y: $0.maxY - 8) })
        let changed = try #require(live.field(form: nil, name: "x")?.array)
        #expect(changed.count == 1 && changed.first?.number != 1)
    }

    @Test(arguments: ["RadioGroup", "CheckBoxGroup", "Chips", "OptionCards"], [100_000, 100_001])
    func longOptionListsDrawLazilyAndFirstChoiceWritesItsShape(_ type: String, _ count: Int) async throws {
        let tests = NativeUITests.Issue71()
        let field = try #require(NativeUITests.Issue71.fields.first { $0.type == type })
        let live = try await tests.program(field)
        let state = live.state
        let version = live.version
        let input = try #require(element(live, at: "/fields/0"))
        guard case let .list(items)? = input.props.first(where: { $0.name == "items" })?.value,
              case let .element(sample)? = items.first else { Issue.record("missing choice"); return }
        let largeItems: [PropValue] = (0..<count).map { index in
            var item = sample
            let key = index == 0 ? "a" : "item-\(index)"
            for i in item.props.indices where item.props[i].name == (type == "CheckBoxGroup" ? "name" : "value") {
                item.props[i].value = .scalar(.string(key))
            }
            return .element(item)
        }
        // Public native-view hook supplies data beyond the core's independent
        // program/result limits. Options have distinct keys, not duplicates.
        let window = await Window(live, component: { element, path in
            guard element.typeName == type else { return nil }
            var large = element
            for i in large.props.indices where large.props[i].name == "items" { large.props[i].value = .list(largeItems) }
            let drawn = NativeComponents.view(for: large, path: path)
            let accepted = drawn != nil
            #expect(accepted, "\(type) refused \(count) supplied options")
            return drawn
        })
        defer { window.close() }
        #expect(live.state == state && live.version == version)
        try await window.click("/fields/0", at: { CGPoint(x: $0.minX + 8, y: $0.minY + 9) })
        if type == "CheckBoxGroup" {
            guard case let .object(entries)? = live.field(form: "f", name: "x") else { Issue.record("missing checkbox record"); return }
            #expect(entries.count == count && Set(entries.map(\.key)).count == count)
            #expect(entries.first?.key == "a" && entries.first?.value == .bool(true))
            #expect(entries.dropFirst().allSatisfy { $0.value == .bool(false) })
        } else { #expect(live.field(form: "f", name: "x") == field.filled) }
        #expect(live.failure == nil)
    }

    @Test(arguments: ["discrete", "continuous"])
    func invalidSliderArithmeticDrawsFallbackWithoutChangingStoredValues(_ variant: String) async throws {
        let (core, library, spec) = try await fieldLibrary()
        let live = try LiveProgram(library: library, promptSpec: spec)
        let original = try await core.parseOneShot(#"root = Slider("x", "discrete", 0, 10, 1, [2])"#.replacingOccurrences(of: "discrete", with: variant), library: library)
        try live.show(original)
        try live.setField(form: nil, name: "x", to: .array([.number(2)]))
        for (key, value) in [("max", -1.0), ("max", 1e-320), ("step", Double.nan), ("step", Double.infinity), ("step", -1.0), ("step", 0.0), ("defaultValue", Double.nan), ("defaultValue", Double.infinity)] {
            let state = live.state
            let version = live.version
            let window = await Window(live, component: { element, path in
                var invalid = element
                for i in invalid.props.indices {
                    if invalid.props[i].name == key { invalid.props[i].value = .scalar(.number(value)) }
                    if key == "defaultValue", invalid.props[i].name == "name" { invalid.props[i].value = .scalar(.string("invalid-value")) }
                }
                let isNative = NativeComponents.view(for: invalid, path: path) != nil
                #expect(isNative == (key == "defaultValue"))
                return AnyView(ElementView(element: invalid).environment(\.elementDrawing, ElementDrawing()))
            })
            #expect(live.state == state && live.version == version)
            try await window.click("/")
            #expect(live.state == state && live.version == version)
            window.close()
        }
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/", at: { CGPoint(x: $0.minX + $0.width * 0.73, y: $0.maxY - 8) })
        #expect(live.field(form: nil, name: "x")?.array?.count == 1)
        #expect(live.field(form: nil, name: "x") != .array([.number(2)]))
    }

    @Test func extremeTextRowsKeepTheirValueAndCanBeEdited() async throws {
        let (core, library, spec) = try await fieldLibrary()
        for rows in ["1e15", "-1", "1e309"] {
            let live = try LiveProgram(library: library, promptSpec: spec)
            try live.show(try await core.parseOneShot("root = TextArea(\"x\", \"Write\", \(rows))", library: library))
            try live.setField(form: nil, name: "x", to: .string("before"))
            let state = live.state
            let version = live.version
            let window = await Window(live)
            defer { window.close() }
            #expect(live.state == state && live.version == version)
            func editors(_ view: NSView) -> [NSTextView] { (view as? NSTextView).map { [$0] } ?? view.subviews.flatMap(editors) }
            let editor = try #require(editors(window.host).first)
            #expect(window.window.makeFirstResponder(editor))
            editor.insertText("after", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
            await window.settle()
            #expect(live.field(form: nil, name: "x") == .string("after"))
        }
    }

    @Test func aContinuousMidpointUsesJavaScriptStepRounding() async throws {
        let (core, library, spec) = try await fieldLibrary()
        let live = try LiveProgram(library: library, promptSpec: spec)
        try live.show(try await core.parseOneShot(#"root = Slider("x", "continuous", 0, 1, 0.2, [0])"#, library: library))
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/", at: { CGPoint(x: $0.midX, y: $0.maxY - 8) })
        #expect(live.field(form: nil, name: "x") == .array([.number(0.6)]))
    }

    @Test func continuousSlidersWithoutAProgramStepKeepFractionalPicks() async throws {
        let (core, library, spec) = try await fieldLibrary()
        let live = try LiveProgram(library: library, promptSpec: spec)
        try live.show(try await core.parseOneShot(#"root = Slider("x", "continuous", 0, 10)"#, library: library))
        let window = await Window(live)
        defer { window.close() }
        try await window.click("/", at: { CGPoint(x: $0.minX + $0.width * 0.731, y: $0.maxY - 8) })
        let value = try #require(live.field(form: nil, name: "x")?.array?.first?.number)
        #expect(value > 0 && value < 10 && value != value.rounded())
    }

    @Test func sixteenSliderValuesDrawAndAnEditKeepsAllThumbs() async throws {
        let (core, library, spec) = try await fieldLibrary()
        let values = JSONValue.array((0..<16).map { .number(Double($0)) })
        let live = try LiveProgram(library: library, promptSpec: spec)
        try live.show(try await core.parseOneShot("root = Slider(\"x\", \"discrete\", 0, 1000, 1, \(values.compact))", library: library))
        let state = live.state
        let window = await Window(live)
        defer { window.close() }
        #expect(live.state == state)
        try await window.click("/", at: { CGPoint(x: $0.minX + $0.width * 0.73, y: $0.minY + 40) })
        let changed = try #require(live.field(form: nil, name: "x")?.array)
        #expect(changed.count == 16)
        #expect(changed != values.array)
        #expect(changed.compactMap(\.number) == changed.compactMap(\.number).sorted())
    }


}

@Suite @MainActor struct Round2Dates {
    @Test func pickingADayStoresLocalMidnightInANonUTCTimeZone() async throws {
        let (core, library, spec) = try await fieldLibrary()
        let live = try LiveProgram(library: library, promptSpec: spec)
        try live.show(try await core.parseOneShot(#"root = DatePicker("x", "single")"#, library: library))
        let zone = try #require(TimeZone(identifier: "Asia/Kolkata"))
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        let drawingCalendar = calendar
        let window = await Window(live, component: { element, path in
            NativeComponents.view(for: element, path: path).map { AnyView($0.environment(\.calendar, drawingCalendar).environment(\.timeZone, zone)) }
        })
        defer { window.close() }
        func pickers(_ view: NSView) -> [NSDatePicker] { (view as? NSDatePicker).map { [$0] } ?? view.subviews.flatMap(pickers) }
        let picker = try #require(pickers(window.host).first)
        // NSDatePicker's date-only editor can retain an invisible time. A
        // picked day must normalize this, not serialize the hidden 15:42.
        picker.dateValue = try #require(calendar.date(from: DateComponents(year: 2026, month: 11, day: 17, hour: 15, minute: 42)))
        #expect(picker.sendAction(picker.action, to: picker.target))
        await window.settle()
        let expected = try #require(try NativeUITests.Issue71().observations().first { $0["scenario"] == .string("localMidnight") }?["value"])
        #expect(expected == .string("2026-11-16T18:30:00.000Z"))
        #expect(live.field(form: nil, name: "x") == expected)
    }
}
}
