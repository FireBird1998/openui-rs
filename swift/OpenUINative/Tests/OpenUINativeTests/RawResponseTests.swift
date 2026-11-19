import OpenUIFFI
@testable import OpenUINative
import Synchronization
import Testing

@Suite(.timeLimit(.minutes(1))) @MainActor struct RawResponses {
    private let schema = #"{"$defs":{"Text":{"properties":{"text":{}}},"Button":{"properties":{"label":{},"action":{}}}}}"#
    private let header = "]]>openui:content?library=bench&name=City+desk\n"
    private let section = "\n]]>openui:scripts {\"count\":1}\n"
    private let script = #"[{"name":"edition","code":"v1"}]"#
    private let query = #"q=Query("edition",{},"default")"#

    // Dispatch creates the tasks synchronously. These providers return immediately.
    private func delivered(_ live: LiveProgram) async {
        for task in Array(live.queryDriver?.calls.values ?? [:].values) { await task.value }
    }

    @Test func bareResponseMatchesStreamSetAndFinishAndDrawsTheToolOutcome() async throws {
        let core = Core()
        let library = try await core.library(schema: schema)
        let live = try LiveProgram(library: library, root: "Text")
        let parser = try await core.session(library: library, root: "Text")
        let text = query + "\nview=Text(q)\nother=Text($name"
        let calls = Mutex<[String]>([])
        try live.attachToolProvider(clock: ManualClock()) { name, _ in
            calls.withLock { $0.append(name) }
            return .string("edition one")
        }
        let preview = try await live.show(response: text, streaming: true)
        #expect(preview.json == (try await parser.set(text)).json)
        #expect(!live.acceptsInteractions)
        await delivered(live)
        #expect(calls.withLock { $0.isEmpty })
        let end = try await live.show(response: text, streaming: false)
        #expect(end.json == (try await parser.finish()).json)
        #expect(end.json != preview.json)
        await delivered(live)
        #expect(calls.withLock { $0 } == ["edition"])
        #expect(drawnText(live) == "edition one")
        #expect(live.acceptsInteractions)
    }

    @Test func winningMessageHidesPartialMarkersAndPublishesItsOwnMetadataAndQuery() async throws {
        let core = Core()
        let library = try await core.library(schema: schema)
        let live = try LiveProgram(library: library)
        try live.attachToolProvider(clock: ManualClock()) { name, _ in .string(name) }
        let parser = try await core.session(library: library)
        try await live.show(response: String(header.dropLast()), streaming: true)
        #expect(live.root == nil && live.response?.name == nil)
        let first = header + #"root=Text("before")"#
        let held = try await live.show(response: first + "]]>openui:con", streaming: true)
        #expect(held.json == (try await parser.set(#"root=Text("before")"#)).json)
        #expect(held.program == #"root=Text("before")"#)
        #expect(drawnText(live) == "before")
        let newHeader = "]]>openui:content?name=Night+desk&library=local%2F1\n"
        let program = query + "\nroot=Text(q)"
        let end = try await live.show(response: first + newHeader + program +
            "\n]]>openui:context\n{\"region\":7}\n]]>openui:end", streaming: false)
        _ = try await parser.set(program)
        #expect(end.json == (try await parser.finish()).json)
        #expect(end.program == program)
        #expect(live.response?.name == "Night desk")
        #expect(live.response?.attributes == ["name":"Night desk", "library":"local/1"])
        #expect(live.response?.context["region"] == .number(7))
        await delivered(live)
        #expect(drawnText(live) == "edition")
        try live.show(try await core.parseOneShot(#"root=Text("plain")"#, library: library))
        #expect(live.response == nil)
    }

    @Test func scriptsStayOutsideTheParserAndOnlyAChangedSettledScriptRefetches() async throws {
        let core = Core()
        let library = try await core.library(schema: schema)
        let live = try LiveProgram(library: library)
        let parser = try await core.session(library: library)
        let calls = Mutex<[String]>([])
        try live.attachToolProvider(clock: ManualClock()) { name, _ in
            let count = calls.withLock { $0.append(name); return $0.filter { $0 == name }.count }
            return .string("\(name)\(count)")
        }
        let program = query + "\nr=Query(\"desk\",{})\nroot=Text(q + \"/\" + r)"
        let raw = header + program
        for suffix in ["\n]]>openui:scr", section, section + "[{", section + script + "\n]]>openui:end\n"] {
            let step = try await live.show(response: raw + suffix, streaming: true)
            #expect(step.program == program)
            #expect(step.json == (try await parser.set(program)).json)
            #expect(live.isQueryLoading && live.parserError == nil && live.response?.error == nil)
            #expect(!live.acceptsInteractions)
            await delivered(live)
            #expect(calls.withLock { $0.isEmpty })
        }
        try await live.show(response: raw + section + script, streaming: false)
        await delivered(live)
        #expect(drawnText(live) == "edition1/desk1")
        let edit = raw + section + script.replacingOccurrences(of: "v1", with: "v2")
        try await live.show(response: edit, streaming: true)
        try live.retryFailedQueries()
        await delivered(live)
        #expect(drawnText(live) == "edition1/desk1")
        #expect(calls.withLock { $0.count } == 2)
        try await live.show(response: edit, streaming: false)
        await delivered(live)
        #expect(drawnText(live) == "edition2/desk1")
        #expect(!live.isQueryLoading && calls.withLock { $0.count } == 3)
    }

    @Test func invalidBundleKeepsDrawingAndBlocksActionsAndRetryAcrossRedraws() async throws {
        let core = Core()
        var events: [String] = []
        let live = try LiveProgram(library: await core.library(schema: schema)) { events.append($0.type) }
        let calls = Mutex(0)
        try live.attachToolProvider(clock: ManualClock()) { _, _ in calls.withLock { $0 += 1 }; return .null }
        let raw = header + query + "\nroot=Button(\"Send\", @ToAssistant(\"sent\"))" + section
        try await live.show(response: raw + script, streaming: false)
        await delivered(live)
        try await live.show(response: raw + "[", streaming: true)
        try live.run("action", of: #require(live.root), message: "Send")
        #expect(events.isEmpty)
        try await live.show(response: raw + "[]", streaming: false)
        try live.set("$edit", to: .number(1))
        #expect(drawnText(live) == "Send")
        #expect(live.response?.error == "Invalid scripts bundle")
        #expect(live.parserError?["message"] == .string("Invalid scripts bundle"))
        #expect(live.parserError?["code"] == .string("parse-failed"))
        #expect(live.parserError?["source"] == .string("parser"))
        #expect(!live.isStreaming && !live.acceptsInteractions)
        let button = try #require(live.root)
        try live.run("action", of: button, message: "Send")
        try live.retryFailedQueries()
        await delivered(live)
        #expect(events.isEmpty && calls.withLock { $0 } == 1)
        try await live.show(response: raw + script, streaming: false)
        #expect(live.parserError == nil && live.acceptsInteractions)
        try live.run("action", of: #require(live.root), message: "Send")
        #expect(events == ["continue_conversation"])
    }

    @Test func clearingAppStreamingAndDiscardingABundlePreviewRestoreProviderFreeActions() async throws {
        let core = Core()
        var events: [String] = []
        let live = try LiveProgram(library: await core.library(schema: schema)) { events.append($0.type) }
        let program = query + "\nroot=Button(\"Send\", @ToAssistant(\"sent\"))"
        let raw = header + program + section + script
        live.isStreaming = true
        try await live.show(response: raw, streaming: false)
        live.isStreaming = false
        try live.run("action", of: #require(live.root), message: "Send")
        #expect(events.count == 1 && !live.isQueryLoading)

        try await live.show(response: program, streaming: false)
        try await live.show(response: raw, streaming: true)
        try live.set("$edit", to: .number(1))
        live.discardPreview()
        try live.run("action", of: #require(live.root), message: "Send")
        #expect(events.count == 2 && !live.isQueryLoading)
    }

    @Test func discardingAnInFlightReadPreventsItFromReplacingTheDrawing() async throws {
        let core = Core()
        let live = try LiveProgram(library: await core.library(schema: schema))
        try await live.show(response: header + #"root=Text("City")"#, streaming: false)
        let edit = Task.immediate { try await live.show(response: #"root=Text("Late")"#, streaming: true) }
        live.discardPreview()
        await #expect(throws: CancellationError.self) { try await edit.value }
        #expect(drawnText(live) == "City" && live.response?.name == "City desk")
    }

    @Test func oversizedRawResponsePreservesThePreviousDrawingAndMetadata() async throws {
        let core = Core()
        let live = try LiveProgram(library: await core.library(schema: schema))
        let raw = header + #"root=Text("City")"#
        try await live.show(response: raw, streaming: false)
        let oversized = raw + section + String(repeating: " ", count: 1 << 20)
        do {
            try await live.show(response: oversized, streaming: false)
            Issue.record("The oversized response was accepted")
        } catch OpenUiError.LimitExceeded(let reason) {
            #expect(reason == "response bundle exceeds the program-size limit")
        }
        #expect(drawnText(live) == "City" && live.response?.name == "City desk")
    }
}
