import Foundation
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

    /// Runs `work` on the core thread and waits for it.
    func run<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            queue.add { continuation.resume(with: Result { try work() }) }
        }
    }

    /// Loads a component library from its JSON Schema.
    public func library(schema: String) async throws -> Library {
        try await run { try Library(schema: schema) }
    }

    /// Parses a whole program one-shot.
    public func parseOneShot(_ program: String, library: Library, root: String? = nil) async throws -> Step {
        try await run {
            try Step.measure(chunks: 1) {
                Core.assertOnCoreThread()
                return OpenUIFFI.parse(program: program, library: library.inner, rootName: root)
            }
        }
    }

    /// The system prompt for a prompt spec (`{library, promptOptions}` as JSON text).
    public func systemPrompt(spec: String) async throws -> String {
        try await run {
            Core.assertOnCoreThread()
            return try OpenUIFFI.generateSystemPrompt(spec: spec)
        }
    }

    /// Merges an edit-mode patch into a program by statement name.
    public func merge(existing: String, patch: String) async throws -> String {
        try await run {
            Core.assertOnCoreThread()
            return OpenUIFFI.mergeStatements(existing: existing, patch: patch, rootId: nil)
        }
    }

    /// Merges the patch so far into `base` and parses the result one-shot: one
    /// step of an edit-mode stream, as a renderer shows it.
    public func mergeAndParse(base: String, patch: String, library: Library, root: String?, chunks: Int) async throws -> Step {
        try await run {
            Core.assertOnCoreThread()
            let merged = OpenUIFFI.mergeStatements(existing: base, patch: patch, rootId: nil)
            var step = try Step.measure(chunks: chunks) {
                OpenUIFFI.parse(program: merged, library: library.inner, rootName: root)
            }
            step.program = merged
            return step
        }
    }

    /// Starts a streaming parse.
    public func session(library: Library, root: String? = nil) async throws -> Session {
        try await run { Session(core: self, library: library, root: root) }
    }
}

/// A component library loaded into the core, shared by every parse against it.
public struct Library: Sendable {
    let inner: ComponentLibrary

    /// Loads the library; throws if the schema isn't JSON. Core thread only.
    init(schema: String) throws {
        Core.assertOnCoreThread()
        inner = try ComponentLibrary(schema: schema)
    }
}

/// A streaming parse in the core. Every call runs on the core thread.
public final class Session: Sendable {
    let core: Core
    let library: Library
    let root: String?
    let inner: StreamSession

    fileprivate init(core: Core, library: Library, root: String?) {
        Core.assertOnCoreThread()
        self.core = core
        self.library = library
        self.root = root
        inner = StreamSession(library: library.inner, rootName: root)
    }

    /// Pushes chunks and returns the result after the last; no chunks gives
    /// the current result.
    public func push(_ chunks: [String]) async throws -> Step {
        try await core.run { [inner] in
            try Step.measure(chunks: chunks.count) {
                Core.assertOnCoreThread()
                return chunks.count == 1 ? inner.push(chunk: chunks[0]) : inner.pushAll(chunks: chunks)
            }
        }
    }

    /// The result once the stream has ended, after its last chunk: a `$name`
    /// at the very end reads as the whole name.
    public func finish() async throws -> Step {
        try await core.run { [inner] in
            try Step.measure(chunks: 0) {
                Core.assertOnCoreThread()
                return inner.finish()
            }
        }
    }

    /// Gives the whole text so far.
    public func set(_ text: String) async throws -> Step {
        try await core.run { [inner] in
            try Step.measure(chunks: 1) {
                Core.assertOnCoreThread()
                return inner.set(text: text)
            }
        }
    }
}

/// A parse result and what producing it cost.
public struct Step: Sendable {
    public var result: ParseResult
    /// The JSON as it crossed, for comparing results exactly.
    public var json: String
    public var timings: Timings
    /// In edit mode, the merged program this step parsed.
    public var program: String?

    /// Calls the core, then decodes, timing each part. Core thread only.
    static func measure(chunks: Int, _ call: () -> ParseOutput) throws -> Step {
        let started = ContinuousClock.now
        let output = call()
        let called = ContinuousClock.now
        let result = try ParseResult(json: output.json, depthLimited: output.depthLimited, sizeLimited: output.sizeLimited)
        let decoded = ContinuousClock.now
        let parse = Duration.nanoseconds(Int64(clamping: output.parseNanos))
        return Step(result: result, json: output.json, timings: Timings(
            chunks: chunks,
            parse: parse,
            crossing: (called - started) - parse,
            decode: decoded - called
        ))
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
