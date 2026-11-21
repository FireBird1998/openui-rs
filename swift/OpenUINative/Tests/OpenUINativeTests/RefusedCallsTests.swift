import AppKit
import Foundation
import OpenUINative
import Testing

private func publicValue(_ value: PropValue) -> JSONValue {
    switch value {
    case let .element(element): return publicElement(element)
    case let .list(items): return .array(items.map(publicValue))
    case let .record(props): return .object(props.map { ($0.name, publicValue($0.value)) })
    case let .binding(marker): return marker.json
    case let .plan(plan): return .array(plan.steps)
    case let .expression(json), let .scalar(json): return json
    }
}
private func publicElement(_ element: Element) -> JSONValue {
    .object([("type", .string(element.typeName)), ("path", element.path.map(JSONValue.string) ?? .null),
             ("partial", .bool(element.partial)), ("dynamic", .bool(element.hasDynamicProps)),
             ("id", element.statementId.map(JSONValue.string) ?? .null),
             ("props", .object(element.props.map { ($0.name, publicValue($0.value)) }))])
}

/// Every readable public LiveProgram observable, plus the field/value readers
/// and authority of a retained public element. Callback/validation registrations
/// are exercised by a mounted click after the snapshot comparison.
@MainActor private struct PublicSnapshot: Equatable {
    var root: JSONValue?
    var errors: [RuntimeIssue]
    var state: JSONValue
    var version: UInt64
    var sizeLimited: Bool
    var depthLimited: Bool
    var failure: String?
    var evaluation: Duration?
    var streaming: Bool
    var requested: Bool
    var accepts: Bool
    var pristine: Set<String>
    var field: JSONValue?
    var value: JSONValue?
    var current: Bool

    init(_ live: LiveProgram, retained: Element) {
        root = live.root.map(publicElement)
        errors = live.errors; state = live.state; version = live.version
        sizeLimited = live.sizeLimited; depthLimited = live.depthLimited
        failure = live.failure; evaluation = live.evaluation
        streaming = live.isStreaming; requested = live.isStreamingRequested
        accepts = live.acceptsInteractions; pristine = live.pristineNames
        field = live.field(form: "f", name: "x"); value = live.value(of: "$n")
        current = live.isCurrent(retained)
    }
}

extension NativeUITests {
@Suite @MainActor struct RefusedPublicCalls {
    @Test(arguments: ["preview show", "complete show", "set", "setField", "write", "enter", "run", "resume"], [false, true])
    func aRefusedEntryPreservesEveryObservableAndItsClickGate(_ entry: String, _ requested: Bool) async throws {
        let core = Core()
        let schema = liveSchema.replacingOccurrences(of: "\"$defs\": {", with: #""$defs": {"CustomField":{"properties":{"name":{},"rules":{}}},"#)
        let library = try await core.library(schema: schema)
        var events: [HostEvent] = []
        var validations = 0
        let live = try LiveProgram(library: library, bindings: [("Input", "value")]) { events.append($0) }
        live.registerValidationComponent("CustomField") { _, _, _ in validations += 1; return .string("valid") }
        let source = #"""
        $n = 0
        root = Form("f", [Input("x", $n), CustomField("custom", {required: true})], Buttons([Button("Go", Action([@Set($n, $n + 1), @ToAssistant("sent")]))]))
        """#
        let complete = try await core.parseOneShot(source, library: library)
        try live.show(complete)
        try live.setField(form: "f", name: "x", to: .string("saved"))
        let root = try #require(live.root)
        let input = try #require(element(live, at: "/fields/0"))
        guard case let .binding(marker)? = input.props.first(where: { $0.name == "value" })?.value else { Issue.record("missing marker"); return }
        let preview = try await (try await core.session(library: library)).push([source + "\n$new = 1"])
        var foreignEvents: [HostEvent] = []
        let foreign = try LiveProgram(library: library) { foreignEvents.append($0) }
        try foreign.show(try await core.parseOneShot(#"""
        m = Mutation("t")
        root = Button("Wait", @Run(m))
        """#, library: library))
        try foreign.run("action", of: try #require(foreign.root), message: "Wait")
        let continuation = try #require(foreignEvents.last)
        let huge = JSONValue.string(String(repeating: "x", count: 9 << 20))
        let window = await Window(live)
        defer { window.close() }
        live.isStreaming = requested
        let before = PublicSnapshot(live, retained: root)
        #expect(throws: (any Error).self) {
            switch entry {
            case "preview show": try live.show(preview, persisted: .string("invalid"))
            case "complete show": try live.show(complete, persisted: .string("invalid"))
            case "set": try live.set("$big", to: huge)
            case "setField": try live.setField(form: "new-form", name: "big", to: huge)
            case "write": try live.write(marker, huge)
            case "enter": try live.enter(huge, into: input, form: "f")
            case "run": try live.run("value", of: input, message: "invalid", form: "f")
            default: try live.resume(continuation, succeeded: true)
            }
        }
        #expect(PublicSnapshot(live, retained: root) == before)
        #expect(events.isEmpty && validations == 0)
        try await window.click("/buttons/buttons/0")
        if requested {
            #expect(PublicSnapshot(live, retained: root) == before)
            #expect(events.isEmpty && validations == 0)
            live.isStreaming = false
            await window.settle()
            try await window.click("/buttons/buttons/0")
        }
        #expect(live.value(of: "$n") == .number(1))
        // Also detect writes hidden by a refused call's cached display.
        if case let .object(entries) = before.state {
            #expect(live.state == .object(entries.map { ($0.key, $0.key == "$n" ? .number(1) : $0.value) }))
        }
        #expect(live.version == before.version + 1)
        #expect(events.count == 1 && validations > 0)
        #expect(events.first?.humanFriendlyMessage == "sent")
    }

    @Test(arguments: ["state", "field"])
    func admittedAppWritesAreNotRefusedByAnOversizedPreview(_ kind: String) async throws {
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let live = try LiveProgram(library: library)
        let source = "$n = 0\nroot = Button(\"Go\", @Set($n, $n + 1))"
        try live.show(try await core.parseOneShot(source, library: library))
        let original = try #require(live.root)
        let stream = try await core.session(library: library)
        let preview = try await stream.push([source])
        try live.show(preview, persisted: .object([("$padding", .string(String(repeating: "p", count: 7 << 20)))]))
        let value = JSONValue.string(String(repeating: "e", count: 2 << 20))
        // The complete store has room; only the disposable preview is full.
        if kind == "state" { try live.set("$edited", to: value) }
        else { try live.setField(form: "f", name: "x", to: value) }
        #expect(live.failure?.hasPrefix("preview projection:") == true)
        #expect(live.isStreaming && !live.acceptsInteractions)
        live.discardPreview()
        let stored = kind == "state" ? live.value(of: "$edited") : live.field(form: "f", name: "x")
        let accepted = stored == value
        #expect(accepted)
        try live.run("action", of: original, message: "Go")
        #expect(live.value(of: "$n") == .number(1))
    }

    @Test func temporarySuspensionsReleaseOnlyTheirOwnGate() async throws {
        let live = try await live("$n = 0\nroot = Button(\"Go\", @Set($n, $n + 1))")
        let button = try #require(live.root)
        let first = live.suspendInteractions()
        let second = live.suspendInteractions()
        #expect(live.isStreaming && !live.isStreamingRequested)
        let before = PublicSnapshot(live, retained: button)
        #expect(throws: OpenUIError.self) { try live.run("action", of: button, message: "Go") }
        #expect(PublicSnapshot(live, retained: button) == before)
        first.release(); first.release()
        #expect(live.isStreaming && !live.isStreamingRequested)
        live.isStreaming = true
        second.release()
        #expect(live.isStreaming && live.isStreamingRequested)
        live.isStreaming = false
        try live.run("action", of: button, message: "Go")
        #expect(live.value(of: "$n") == .number(1))
    }

    @Test(arguments: [false, true])
    func refusedShowsPreserveAnExistingPreviewAndTheAppsOwnRequest(_ requested: Bool) async throws {
        let core = Core()
        let library = try await core.library(schema: liveSchema)
        let live = try LiveProgram(library: library)
        let source = "$n = 0\nroot = Button(\"Go\", @Set($n, $n + 1))"
        let complete = try await core.parseOneShot(source, library: library)
        try live.show(complete)
        let original = try #require(live.root)
        let stream = try await core.session(library: library)
        let preview = try await stream.push([source + "\n$new = 1"])
        try live.show(preview)
        let drawn = try #require(live.root)
        live.isStreaming = requested
        for candidate in [preview, complete] {
            let before = PublicSnapshot(live, retained: drawn)
            #expect(throws: (any Error).self) { try live.show(candidate, persisted: .string("invalid")) }
            #expect(PublicSnapshot(live, retained: drawn) == before)
            #expect(throws: OpenUIError.self) { try live.run("action", of: drawn, message: "Go") }
            #expect(PublicSnapshot(live, retained: drawn) == before)
        }
        live.discardPreview()
        #expect(live.isStreamingRequested == requested)
        #expect(live.isStreaming == requested)
        if requested { #expect(throws: OpenUIError.self) { try live.run("action", of: original, message: "Go") } }
        live.isStreaming = false
        try live.run("action", of: original, message: "Go")
        #expect(live.value(of: "$n") == .number(1))
    }
}
}
