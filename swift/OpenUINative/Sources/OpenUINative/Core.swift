import Foundation
import Synchronization
import OpenUIFFI

/// The one thread the core runs on. It has the stack the core needs (at least
/// 512 KB; 1 MB here), and nothing else ever calls the core: in debug builds,
/// a call from any other thread stops the app.
public final class Core: Sendable {
    public static let stackSize = 1 << 20

    private let queue = JobQueue()

    public init() {
        let queue = self.queue
        let thread = Thread {
            Thread.current.threadDictionary[Core.marker] = true
            queue.run()
        }
        thread.name = "openui-core"
        thread.stackSize = Core.stackSize
        thread.start()
    }

    deinit {
        queue.stop()
    }

    static let marker = "dev.openui-rs.viewer.core"

    /// Whether the caller is on a core thread.
    static var isCurrent: Bool {
        Thread.current.threadDictionary[marker] as? Bool == true
    }

    static func assertOnCoreThread(_ function: StaticString = #function) {
        assert(isCurrent, "\(function) called the core off its thread")
    }

    /// Runs `work` on the core thread and waits for it. Whatever it throws
    /// comes back as an `OpenUIError`.
    func run<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws(OpenUIError) -> T {
        let result: Result<T, OpenUIError> = await withCheckedContinuation { continuation in
            queue.add { continuation.resume(returning: Result { () throws(OpenUIError) in try bridged(work) }) }
        }
        return try result.get()
    }

    /// Loads a component library from its JSON Schema; `.invalidSchema` if
    /// the schema isn't JSON.
    public func library(schema: String) async throws(OpenUIError) -> Library {
        try await run { try Library(core: self, schema: schema) }
    }

    /// Parses a whole program one-shot.
    public func parseOneShot(_ program: String, library: Library, root: String? = nil) async throws(OpenUIError) -> Step {
        try await run {
            try Step.measure(chunks: 1) {
                Core.assertOnCoreThread()
                return OpenUIFFI.parse(program: program, library: library.inner, rootName: root ?? library.root)
            }
        }
    }

    /// The system prompt for a prompt spec given as JSON text, exactly as
    /// upstream's `generateSystemPrompt` takes it: `{library, promptOptions}`,
    /// `{cloud: true, ...}` or a bare prompt spec. `.invalidPromptSpec` if
    /// it isn't JSON, `.prompt` with upstream's message if upstream would
    /// throw. `systemPrompt(library:options:)` and
    /// `cloudConfigBlock(library:options:instructions:)` build the spec
    /// from typed options.
    public func systemPrompt(spec: String) async throws(OpenUIError) -> String {
        try await run {
            Core.assertOnCoreThread()
            return try OpenUIFFI.generateSystemPrompt(spec: spec)
        }
    }

    /// Merges an edit-mode patch into a program by statement name.
    public func merge(existing: String, patch: String) async throws(OpenUIError) -> String {
        try await run {
            Core.assertOnCoreThread()
            return OpenUIFFI.mergeStatements(existing: existing, patch: patch, rootId: nil)
        }
    }

    /// Merges the patch so far into `base` and parses the result one-shot: one
    /// step of an edit-mode stream, as a renderer shows it.
    public func mergeAndParse(base: String, patch: String, library: Library, root: String?, chunks: Int, streaming: Bool = false) async throws(OpenUIError) -> Step {
        try await run {
            Core.assertOnCoreThread()
            let merged = OpenUIFFI.mergeStatements(existing: base, patch: patch, rootId: nil)
            var step = try Step.measure(chunks: chunks, streaming: streaming) {
                OpenUIFFI.parse(program: merged, library: library.inner, rootName: root ?? library.root)
            }
            step.program = merged
            step.previewEndsAtLineBreak = Step.endsAtLineBreak(merged)
            return step
        }
    }

    /// Starts a streaming parse.
    public func session(library: Library, root: String? = nil) async throws(OpenUIError) -> Session {
        try await run { Session(core: self, library: library, root: root) }
    }
}

/// A component library loaded into the core, shared by every parse against it.
public struct Library: Sendable {
    let core: Core
    let inner: ComponentLibrary
    let root: String?
    let components: [Data: DefinedComponent]
    let functions: [(name: String, callback: any LibraryFunction)]

    /// Loads the library; throws if the schema isn't JSON. Core thread only.
    init(core: Core, schema: String) throws {
        self.core = core
        Core.assertOnCoreThread()
        root = nil
        components = [:]
        functions = []
        inner = try ComponentLibrary(schema: schema)
    }

    init(core: Core, definition: String, components: [DefinedComponent], functions: [DefinedFunction] = [], root: String?) throws {
        Core.assertOnCoreThread()
        self.core = core
        self.root = root
        inner = try ComponentLibrary.fromDefinition(definition: definition)
        self.functions = functions.compactMap { function in
            function.callback.map { (function.name, $0) }
        }
        // Like the core's component record, the last declaration wins. UTF-8
        // keys keep distinct names that Swift String equality would normalize.
        self.components = Dictionary(components.lazy.map { (Data($0.name.utf8), $0) },
                                     uniquingKeysWith: { _, last in last })
    }

    /// A defined library's core-generated prompt spec as JSON text.
    public func promptSpec() async throws(OpenUIError) -> String? {
        try await core.run { inner.promptSpec() }
    }
}

/// A streaming parse in the core. Every call runs on the core thread.
public final class Session: Sendable {
    let core: Core
    let library: Library
    let root: String?
    let inner: StreamSession
    /// The raw response the reader last accepted, as `response` and
    /// `append(response:)` gave it; changed only on the core thread.
    let responseText = Mutex("")
    // On the core thread, like the parser it describes.
    let previewLineBoundary = Mutex(false)

    fileprivate init(core: Core, library: Library, root: String?) {
        Core.assertOnCoreThread()
        self.core = core
        self.library = library
        self.root = root ?? library.root
        inner = StreamSession(library: library.inner, rootName: self.root)
    }

    /// Pushes chunks and returns the result after the last; no chunks gives
    /// the current result.
    public func push(_ chunks: [String]) async throws(OpenUIError) -> Step {
        try await core.run { [self] in
            var step = try Step.measure(chunks: chunks.count, streaming: true) {
                Core.assertOnCoreThread()
                if chunks.count == 1, let chunk = chunks.first { return inner.push(chunk: chunk) }
                return inner.pushAll(chunks: chunks)
            }
            step.previewEndsAtLineBreak = previewLineBoundary.withLock { boundary in
                boundary = Step.endsAtLineBreak(chunks, after: boundary)
                return boundary
            }
            return step
        }
    }

    /// The result once the stream has ended, after its last chunk: a `$name`
    /// at the very end reads as the whole name.
    public func finish() async throws(OpenUIError) -> Step {
        try await core.run { [inner] in
            try Step.measure(chunks: 0) {
                Core.assertOnCoreThread()
                return inner.finish()
            }
        }
    }

    /// Gives the whole text so far.
    public func set(_ text: String) async throws(OpenUIError) -> Step {
        try await core.run { [self] in
            var step = try Step.measure(chunks: 1, streaming: true) {
                Core.assertOnCoreThread()
                return inner.set(text: text)
            }
            step.previewEndsAtLineBreak = Step.endsAtLineBreak(text)
            previewLineBoundary.withLock { $0 = step.previewEndsAtLineBreak }
            return step
        }
    }
}

/// A parse result and what producing it cost.
public struct Step: Sendable {
    /// A push/set result is a preview until finish succeeds. Clearing an app
    /// streaming flag cannot grant a preview interaction authority.
    public let isStreaming: Bool
    public var result: ParseResult
    /// The JSON as it crossed, for comparing results exactly.
    public var json: String
    public var timings: Timings
    /// The merged program for a patch, or the extracted program for a framed response.
    public var program: String?
    /// The reader metadata for a step parsed from a raw response.
    public internal(set) var response: ResponseMetadata?
    /// The parse result itself, for a runtime session to evaluate.
    let parsed: ParsedProgram
    var previewEndsAtLineBreak = false

    /// A syntactically closed final expression can still grow in a prefix.
    /// Only a completed line can be treated as settled while input is arriving.
    static func endsAtLineBreak(_ text: String) -> Bool {
        endsAtLineBreak([text], after: false)
    }

    static func endsAtLineBreak(_ chunks: [String], after previous: Bool) -> Bool {
        let chunkLimit = 4_096
        var remaining = 65_536
        for chunk in chunks.reversed().prefix(chunkLimit) {
            for byte in chunk.utf8.reversed().prefix(remaining) {
                switch byte {
                case 10: return true
                case 9, 13, 32: remaining -= 1
                default: return false
                }
            }
            if remaining == 0 { return false }
        }
        return chunks.count <= chunkLimit && previous
    }

    /// Marks a whole-parser adapter's intermediate result as a streamed
    /// preview without reparsing or granting it interaction authority.
    public func asStreamingPreview() -> Step {
        Step(isStreaming: true, result: result, json: json, timings: timings, program: program, response: response,
             parsed: parsed, previewEndsAtLineBreak: previewEndsAtLineBreak)
    }

    /// Calls the core, then decodes, timing each part. Core thread only.
    static func measure(chunks: Int, streaming: Bool = false, _ call: () -> ParseOutput) throws(OpenUIError) -> Step {
        let started = ContinuousClock.now
        let output = call()
        let called = ContinuousClock.now
        let result = try ParseResult(json: output.json, depthLimited: output.depthLimited, sizeLimited: output.sizeLimited)
        let decoded = ContinuousClock.now
        let parse = Duration.nanoseconds(Int64(clamping: output.parseNanos))
        let timings = Timings(
            chunks: chunks,
            parse: parse,
            crossing: (called - started) - parse,
            decode: decoded - called
        )
        return Step(isStreaming: streaming, result: result, json: output.json, timings: timings, program: nil, parsed: output.program)
    }
}

/// Where one update's time went.
public struct Timings: Sendable, Equatable {
    /// How many chunks the update pushed.
    public var chunks: Int
    /// The core parsing.
    public var parse: Duration
    /// Serializing to JSON and crossing into Swift.
    public var crossing: Duration
    /// Decoding the JSON into the viewer's model.
    public var decode: Duration
    /// Evaluating the tree in a runtime session and decoding it; filled in by
    /// whoever evaluates it.
    public var evaluate: Duration?
    /// From handing the result to the UI until it was drawn; filled in by the UI.
    public var redraw: Duration?
}

/// Jobs for the core thread, run one at a time in order.
private final class JobQueue: @unchecked Sendable {
    private let condition = NSCondition()
    private var jobs: [@Sendable () -> Void] = []
    private var stopped = false

    func add(_ job: @escaping @Sendable () -> Void) {
        condition.lock()
        jobs.append(job)
        condition.signal()
        condition.unlock()
    }

    func stop() {
        condition.lock()
        stopped = true
        condition.signal()
        condition.unlock()
    }

    func run() {
        while true {
            condition.lock()
            while jobs.isEmpty && !stopped { condition.wait() }
            if jobs.isEmpty { condition.unlock(); return }
            let job = jobs.removeFirst()
            condition.unlock()
            job()
        }
    }
}
