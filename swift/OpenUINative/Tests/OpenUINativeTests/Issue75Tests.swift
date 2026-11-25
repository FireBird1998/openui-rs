import AppKit
import Foundation
import OpenUINative
import SwiftUI
import SwiftTestSupport
import Testing

extension NativeUITests {
@Suite(NativeTestWatchdog.native) @MainActor struct Issue75 {
    @Test(arguments: [false, true], ["initialize", "evaluate"])
    func refusedShowsKeepBothSessionsAndLaterAppWritesOnTheOldProgram(_ preview: Bool, _ step: String) async throws {
        let core = Core()
        NativeTestWatchdog.waitingFor("Core to load the live schema")
        let library = try await core.library(schema: liveSchema)
        var events: [HostEvent] = []
        let live = try LiveProgram(library: library) { events.append($0) }
        let source = #"""
$n=1
root=Stack([TextContent("OLD"), Button("Go", Action([@Set($n,$n+1),@ToAssistant("old")]))])
"""#
        NativeTestWatchdog.waitingFor("Core to parse the original program")
        let complete = try await core.parseOneShot(source, library: library)
        try live.show(complete)
        let retained = try #require(element(live, at: "/children/1"))
        if preview { try live.show(complete.asStreamingPreview()) }
        let before = live.state, version = live.version, root = live.root?.props
        NativeTestWatchdog.waitingFor("Core to parse the candidate program")
        let candidate = try await core.parseOneShot(#"""
$n=99
$new=3
$shape={type:"element",typeName:"Stack",props:{},partial:false,hasDynamicProps:false,extra:1}
root=true ? $shape : Stack([])
"""#, library: library)
        let attempted = preview ? candidate.asStreamingPreview() : candidate
        #expect(throws: (any Error).self) {
            try live.show(attempted, persisted: step == "initialize" ? .array([]) : .object([("$extra", .number(8))]))
        }
        #expect(live.state == before && live.version == version)
        #expect(live.root?.props.map(\.name) == root?.map(\.name))
        #expect(live.isStreaming == preview)
        #expect(live.isCurrent(retained) == !preview)
        #expect(live.value(of: "$new") == nil && live.value(of: "$extra") == nil)
        try live.set("$n", to: .number(5))
        if case let .scalar(.string(text))? = element(live, at: "/children/0")?.props.first?.value {
            #expect(text == "OLD")
        } else { Issue.record("old program was replaced") }
        try live.setField(form: "f", name: "x", to: .string("edited"))
        if case let .scalar(.string(text))? = element(live, at: "/children/0")?.props.first?.value {
            #expect(text == "OLD")
        } else { Issue.record("old program was replaced") }
        if preview { live.discardPreview() }
        // App writes redraw, so obtain the current button after verifying the
        // pre-refusal retained button before those writes in complete mode.
        if !preview { #expect(!live.isCurrent(retained)) }
        try live.run("action", of: try #require(element(live, at: "/children/1")), message: "Go")
        #expect(live.value(of: "$n") == .number(6))
        #expect(events.last?.humanFriendlyMessage == "old")
    }

    @Test func persistedValuesUseTheRealPromptAnnotatedSessionsAdmission() async throws {
        var directory = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { directory.deleteLastPathComponent() }
        let core = Core()
        NativeTestWatchdog.waitingFor("Core to load the React UI schema")
        let library = try await core.library(schema: String(contentsOf: directory.appendingPathComponent("conformance/imported/schemas/react-ui.json"), encoding: .utf8))
        let spec = try String(contentsOf: directory.appendingPathComponent("conformance/prompts/react-ui-schema.json"), encoding: .utf8)
        let live = try LiveProgram(library: library, promptSpec: spec)
        NativeTestWatchdog.waitingFor("Core to parse the bound text area")
        let step = try await core.parseOneShot("$v=\"initial\"\nroot=TextArea(\"x\",\"\",3,null,$v)", library: library)
        try live.show(step, persisted: .object([("$v", .string("saved"))]))
        #expect(live.value(of: "$v") == .string("saved"))
        let field = try #require(live.root)
        guard case let .binding(marker)? = field.props.first(where: { $0.name == "value" })?.value else {
            Issue.record("prompt binding was lost"); return
        }
        let state = live.state, version = live.version
        #expect(throws: (any Error).self) { try live.show(step, persisted: .array([])) }
        #expect(live.state == state && live.version == version && live.isCurrent(field))
        try live.write(marker, .string("edited"))
        #expect(live.value(of: "$v") == .string("edited"))
        #expect(!live.pristineNames.contains("$v"))
    }

    @Test(arguments: [false, true], ["single", "range"])
    func unsetDatesDrawAnEmptySystemFieldAndWriteThroughTheBinding(_ bound: Bool, _ mode: String) async throws {
        let original = try #require(Issue71.fields.first { $0.type == "DatePicker" })
        let field = Issue71.Field(type: original.type, source: original.source.replacingOccurrences(of: "single", with: mode), filled: original.filled)
        let live = try await Issue71().program(field, bound: bound)
        let before = live.state, version = live.version
        let zone = try #require(TimeZone(secondsFromGMT: 0))
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        let drawingCalendar = calendar
        let window = await Window(live, component: { element, path in
            guard element.typeName == "DatePicker" else { return nil }
            return NativeComponents.view(for: element, path: path).map {
                AnyView($0.environment(\.calendar, drawingCalendar).environment(\.timeZone, zone))
            }
        })
        defer { window.close() }
        func fields(_ view: NSView) -> [NSTextField] {
            (view as? NSTextField).map { [$0] } ?? view.subviews.flatMap(fields)
        }
        func pickers(_ view: NSView) -> [NSDatePicker] {
            (view as? NSDatePicker).map { [$0] } ?? view.subviews.flatMap(pickers)
        }
        #expect(pickers(window.host).isEmpty)
        let input = try #require(fields(window.host).first { $0.isEditable })
        #expect(input.stringValue.isEmpty)
        #expect(live.state == before && live.version == version)
        let date = try #require(ISO8601DateFormatter().date(from: "2026-11-17T12:00:00Z"))
        let formatter = DateFormatter()
        formatter.calendar = drawingCalendar; formatter.timeZone = zone
        formatter.dateStyle = .short; formatter.timeStyle = .none
        NativeTestWatchdog.waitingFor("AppKit to focus the empty date field and commit the edit")
        #expect(window.window.makeFirstResponder(input))
        let editor = try #require(input.currentEditor())
        editor.insertText(formatter.string(from: date))
        input.validateEditing()
        #expect(window.window.makeFirstResponder(nil))
        await window.settle("the date picker to redraw after committing the edit")
        let value = bound ? live.value(of: "$v") : live.field(form: "f", name: "x")
        let dateValue = mode == "range" ? value?["from"] : value
        #expect(dateValue == .string("2026-11-17T00:00:00.000Z"))
        if mode == "range" { #expect(value?["to"] == nil) }
        #expect(live.version > version)
        #expect(!pickers(window.host).isEmpty)
    }

    @Test(arguments: [false, true])
    func tabSwitchesDuringStreamingAreLocalAndNeverWriteOrRun(_ preview: Bool) async throws {
        let core = Core()
        NativeTestWatchdog.waitingFor("Core to load the tabs schema")
        let library = try await core.library(schema: Issue63.extendedSchema)
        var events: [HostEvent] = []
        let live = try LiveProgram(library: library) { events.append($0) }
        let source = #"root = Form("f", [Tabs([TabItem("First", [Input("a", "ok", {required: true})]), TabItem("Second", [Input("b", null, {required: true})])])], Buttons([Button("Send")]))"#
        NativeTestWatchdog.waitingFor("Core to parse the tabs form")
        try live.show(try await core.parseOneShot(source, library: library))
        let window = await Window(live)
        defer { window.close() }
        if preview {
            NativeTestWatchdog.waitingFor("Core to open the streaming tabs session")
            let stream = try await core.session(library: library)
            NativeTestWatchdog.waitingFor("Core to parse the streaming tabs preview")
            try live.show(try await stream.push([source]))
        } else { live.isStreaming = true }
        await window.settle("the tabs to redraw in streaming mode")
        let before = live.state, version = live.version
        func segments(_ view: NSView) -> [NSSegmentedControl] {
            (view as? NSSegmentedControl).map { [$0] } ?? view.subviews.flatMap(segments)
        }
        let picker = try #require(segments(window.host).first)
        #expect(picker.isEnabled)
        NativeTestWatchdog.waitingFor("AppKit to dispatch the streaming tab selection")
        picker.selectedSegment = 1
        #expect(picker.sendAction(picker.action, to: picker.target))
        await window.settle("the selected streaming tab to redraw")
        #expect(picker.selectedSegment == 1)
        #expect(live.state == before && live.version == version && events.isEmpty)
        try await window.click("/buttons/buttons/0")
        #expect(events.isEmpty && live.state == before && live.version == version)
        if preview { live.discardPreview() } else { live.isStreaming = false }
        await window.settle("the tabs to redraw after streaming ends")
        try await window.click("/buttons/buttons/0")
        #expect(events.count == (preview ? 1 : 0))
    }
    @Test(arguments: [false, true])
    func endingAPreviewRestoresTheCompleteTabsSelectionAndValidation(_ finish: Bool) async throws {
        let core = Core()
        NativeTestWatchdog.waitingFor("Core to load the tabs schema")
        let library = try await core.library(schema: Issue63.extendedSchema)
        var events: [HostEvent] = []
        let live = try LiveProgram(library: library) { events.append($0) }
        let source = #"root = Form("f", [Tabs([TabItem("First", [Input("a", "ok", {required: true})]), TabItem("Second", [Input("b", null, {required: true})])])], Buttons([Button("Send")]))"#
        NativeTestWatchdog.waitingFor("Core to parse the tabs form")
        let complete = try await core.parseOneShot(source, library: library)
        try live.show(complete)
        let window = await Window(live)
        defer { window.close() }
        func segments(_ view: NSView) -> [NSSegmentedControl] {
            (view as? NSSegmentedControl).map { [$0] } ?? view.subviews.flatMap(segments)
        }
        let initialPicker = try #require(segments(window.host).first)
        #expect(initialPicker.selectedSegment == 0)
        let before = live.state, version = live.version
        // Keep exactly the same tree; the preview's end must invalidate the
        // mounted TabsView even though its props have not changed.
        try live.show(complete.asStreamingPreview())
        await window.settle("the preview tabs to redraw")
        let previewState = live.state, previewVersion = live.version
        let previewPicker = try #require(segments(window.host).first)
        NativeTestWatchdog.waitingFor("AppKit to dispatch the preview tab selection")
        previewPicker.selectedSegment = 1
        #expect(previewPicker.sendAction(previewPicker.action, to: previewPicker.target))
        await window.settle("the selected preview tab to redraw")
        #expect(previewPicker.selectedSegment == 1)
        #expect(live.state == previewState && live.version == previewVersion && events.isEmpty)
        if finish { try live.show(complete) } else { live.discardPreview() }
        await window.settle("the complete tabs to redraw after the preview ends")
        #expect(!live.isStreaming)
        let restoredPicker = try #require(segments(window.host).first)
        #expect(restoredPicker.selectedSegment == 0)
        #expect(live.state == before && live.version == version && events.isEmpty)
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1) // The complete First tab is valid.
        let completePicker = try #require(segments(window.host).first)
        NativeTestWatchdog.waitingFor("AppKit to dispatch the complete tab selection")
        completePicker.selectedSegment = 1
        #expect(completePicker.sendAction(completePicker.action, to: completePicker.target))
        await window.settle("the selected complete tab to redraw")
        try await window.click("/buttons/buttons/0")
        #expect(events.count == 1) // The complete Second tab is invalid.
    }

}
}
