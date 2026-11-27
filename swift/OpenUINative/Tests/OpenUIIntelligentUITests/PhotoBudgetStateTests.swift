@testable import OpenUIIntelligentUI
import AppKit
import Synchronization
import SwiftTestSupport
import Testing

/// The photo budget's invariant hook for tests: every failed audit, at the
/// outermost exit of any transition, records an issue.
@MainActor enum PhotoBudgetAudit {
    /// Where the failing step was, for the random sequence test.
    static var context = ""

    static func install() {
        PhotoBudget.onViolation = { violations in
            let place = context.isEmpty ? "" : " at \(context)"
            Issue.record("Photo budget audit failed\(place): \(violations.joined(separator: "; "))")
        }
    }
}

private let mebibyte = 1024 * 1024
// Literals on purpose: a change to the production limits must fail here.
private let slotLimit = 4
private let byteLimit = 32 * mebibyte
private let reservedBytes = 8 * mebibyte

@MainActor private func injectedRaster(bytes: Int) throws -> PhotoRaster {
    let image = try #require(PhotoRaster.bitmap(width: 1, height: 1)?.makeImage())
    return PhotoRaster(first: image, animation: nil, bytes: bytes)
}

/// Counts the calls an injected loader receives.
private final class LoaderCalls: Sendable {
    private let count = Mutex(0)
    var value: Int { count.withLock { $0 } }

    func loader(returning raster: PhotoRaster?) -> @Sendable (PhotoSource, PhotoSize) async -> PhotoRaster? {
        { _, _ in
            self.count.withLock { $0 += 1 }
            return raster
        }
    }
}

/// An injected loader that waits until the test opens it, or until its task
/// is cancelled, which returns nil as `PhotoLoader.load` does.
private final class PhotoGate: Sendable {
    private struct State {
        var waiting: CheckedContinuation<PhotoRaster?, Never>?
        var result: PhotoRaster??
        var calls = 0
    }

    private let state = Mutex(State())
    var calls: Int { state.withLock { $0.calls } }

    var loader: @Sendable (PhotoSource, PhotoSize) async -> PhotoRaster? {
        { _, _ in await self.wait() }
    }

    private func wait() async -> PhotoRaster? {
        await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                let ready: PhotoRaster?? = state.withLock { state in
                    state.calls += 1
                    if let result = state.result { return .some(result) }
                    state.waiting = continuation
                    return nil
                }
                if case .some(let result) = ready { continuation.resume(returning: result) }
            }
        } onCancel: {
            open(nil)
        }
    }

    /// The first call decides the result.
    func open(_ raster: PhotoRaster?) {
        let waiting = state.withLock { state -> CheckedContinuation<PhotoRaster?, Never>? in
            guard state.result == nil else { return nil }
            state.result = .some(raster)
            defer { state.waiting = nil }
            return state.waiting
        }
        waiting?.resume(returning: raster)
    }
}

/// SplitMix64, so a failing seed replays exactly.
private struct SplitMix64 {
    var state: UInt64

    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }

    mutating func below(_ bound: Int) -> Int { Int(next() % UInt64(max(1, bound))) }
}

/// How long a test waits for `reserve()` to admit before it fails by name.
/// A waiter that is never woken (a slot that never returns) would otherwise
/// hang the test until the watchdog.
private let reserveBound = Duration.seconds(5)

/// How long a test waits for a queued waiter's task to end once it is woken
/// or cancelled. A waiter that is never woken would otherwise hang the test
/// until the watchdog.
private let waiterBound = Duration.seconds(5)

@MainActor private final class Flag {
    var value = false
}

/// The first of a task's value and a timer.
@MainActor private final class FirstOf<Value: Sendable> {
    enum Outcome: Sendable { case ended(Value), timedOut }

    private var outcome: Outcome?
    private var waiting: CheckedContinuation<Outcome, Never>?

    func resolve(_ value: Outcome) {
        guard outcome == nil else { return }
        outcome = value
        waiting?.resume(returning: value)
        waiting = nil
    }

    func value() async -> Outcome {
        if let outcome { return outcome }
        return await withCheckedContinuation { waiting = $0 }
    }
}

/// The task's value, or a failure that names `boundName` when the task
/// doesn't end within `bound`. The stuck task is left suspended, not awaited.
@MainActor private func value<Value: Sendable>(
    of task: Task<Value, Never>, within bound: Duration, _ boundName: String, _ what: String
) async throws -> Value {
    let first = FirstOf<Value>()
    let waiting = Task { first.resolve(.ended(await task.value)) }
    let timer = Task {
        try await Task.sleep(for: bound)
        first.resolve(.timedOut)
    }
    defer { timer.cancel() }
    switch await first.value() {
    case .ended(let value):
        withExtendedLifetime(waiting) {}
        return value
    case .timedOut:
        throw CollectionInputFailure(description: "\(what) did not end within \(boundName) (\(bound))")
    }
}

extension PhotoBudget.Scope {
    /// `reserve()`, cancelled at `bound`. Cancelling removes the waiter, so a
    /// waiter that is never admitted fails here, by name, instead of hanging.
    func reserve(within bound: Duration, sourceLocation: SourceLocation = #_sourceLocation) async throws
        -> PhotoBudget.Reservation {
        let timedOut = Flag()
        let reserving = Task { await self.reserve() }
        let timer = Task {
            try await Task.sleep(for: bound)
            timedOut.value = true
            reserving.cancel()
        }
        defer { timer.cancel() }
        let reservation = await reserving.value
        if timedOut.value {
            Issue.record("reserve() was not admitted within reserveBound (\(bound))", sourceLocation: sourceLocation)
        }
        return try #require(reservation, sourceLocation: sourceLocation)
    }
}

extension CollectionDrawingTests {
@Suite(NativeTestWatchdog()) @MainActor struct PhotoBudgetStateTests {
    init() { PhotoBudgetAudit.install() }

    private let size = PhotoSize(points: CGSize(width: 64, height: 64), scale: 1)
    private let offline = PhotoSource("http://127.0.0.1:9/injected")

    private func blankWindow() async throws -> CollectionWindow {
        try await CollectionWindow("root = Text(\"Budget\")", width: 100, height: 120, observeProgramDrawing: true)
    }

    // MARK: The checker

    @Test func auditFindsEachKindOfViolation() {
        typealias Snapshot = PhotoBudget.Snapshot
        let consistent = Snapshot(
            closed: false, active: 2, charged: 2 * reservedBytes + mebibyte, waiters: 1,
            holds: [Snapshot.Hold(stage: .acquired, reserved: reservedBytes, handleRasterBytes: nil),
                    Snapshot.Hold(stage: .loading, reserved: reservedBytes, handleRasterBytes: nil),
                    Snapshot.Hold(stage: .holding(bytes: mebibyte), reserved: reservedBytes, handleRasterBytes: mebibyte)],
            releasedHandlesWithRaster: 0)
        #expect(PhotoBudget.audit(consistent) == [])

        func expectViolation(_ fragment: String, _ change: (inout Snapshot) -> Void,
                             sourceLocation: SourceLocation = #_sourceLocation) {
            var broken = consistent
            change(&broken)
            let violations = PhotoBudget.audit(broken)
            #expect(violations.contains { $0.contains(fragment) }, "\(fragment): \(violations)",
                    sourceLocation: sourceLocation)
        }
        // 1. charged is the sum of the holds' bytes.
        expectViolation("not the holds' sum") { $0.charged += 1 }
        // 2. charged stays within the window allowance.
        expectViolation("outside 0...") {
            $0.holds = Array(repeating: Snapshot.Hold(stage: .holding(bytes: reservedBytes), reserved: reservedBytes,
                                                      handleRasterBytes: nil), count: 5)
            $0.active = 0
            $0.charged = 5 * reservedBytes
        }
        // 3. no hold charges more than it reserved.
        expectViolation("over its reserved") {
            $0.holds[2] = Snapshot.Hold(stage: .holding(bytes: reservedBytes + 1), reserved: reservedBytes,
                                        handleRasterBytes: nil)
            $0.charged = 3 * reservedBytes + 1
        }
        // 4. active counts the acquired and loading holds, and at most four.
        expectViolation("acquired or loading holds") { $0.active = 3 }
        expectViolation("is over") {
            $0.holds = Array(repeating: Snapshot.Hold(stage: .loading, reserved: mebibyte, handleRasterBytes: nil),
                             count: 5)
            $0.active = 5
            $0.charged = 5 * mebibyte
        }
        // 5. a closed budget is empty, and no released or unloaded handle shows a raster.
        expectViolation("closed budget") { $0.closed = true }
        expectViolation("closed budget") {
            $0.closed = true
            $0.holds = []
            $0.active = 0
            $0.charged = 0
        }
        expectViolation("released handles") { $0.releasedHandlesWithRaster = 1 }
        expectViolation("shows a raster before it loaded") { $0.holds[0].handleRasterBytes = mebibyte }
        expectViolation("shows") { $0.holds[2].handleRasterBytes = 2 * mebibyte }
    }

    // MARK: The confirm's attacks, restated

    @Test func loadingAgainOnAHoldingReservationIsRefused() async throws {
        let window = try await blankWindow()
        defer { window.window.closeForTest() }
        let budget = PhotoBudget.inWindow(window.window)
        let scope = budget.makeScope()
        defer { scope.cancel() }
        let size = try #require(size), source = try #require(offline)
        let small = try injectedRaster(bytes: mebibyte)
        var holding: [PhotoBudget.Reservation] = []
        for _ in 0..<4 {
            let reservation = try await scope.reserve(within: reserveBound)
            #expect(await reservation.load(source, size: size, loader: { _, _ in small }))
            #expect(reservation.raster?.bytes == mebibyte)
            holding.append(reservation)
        }
        #expect(budget.charged == 4 * mebibyte)
        #expect(budget.active == 0)
        // The freed allowance admits three more full reservations.
        var acquired: [PhotoBudget.Reservation] = []
        for _ in 0..<3 { acquired.append(try await scope.reserve(within: reserveBound)) }
        #expect(budget.charged == 28 * mebibyte)
        #expect(budget.active == 3)

        // A holding reservation's slot is spent: another load makes no request
        // and changes nothing, whatever the raster would have been.
        let calls = LoaderCalls()
        let full = try injectedRaster(bytes: reservedBytes)
        for reservation in holding {
            #expect(await reservation.load(source, size: size, loader: calls.loader(returning: full)) == false)
            #expect(reservation.raster?.bytes == mebibyte)
        }
        #expect(calls.value == 0)
        #expect(budget.charged == 28 * mebibyte)
        #expect(budget.active == 3)
        #expect(budget.audit() == [])
        withExtendedLifetime(acquired) {}
    }

    @Test(arguments: [64 * 1024 * 1024, 8 * 1024 * 1024 + 1])
    func aRasterLargerThanItsReservationIsRefusedAndReleased(_ bytes: Int) async throws {
        let window = try await blankWindow()
        defer { window.window.closeForTest() }
        let budget = PhotoBudget.inWindow(window.window)
        let scope = budget.makeScope()
        defer { scope.cancel() }
        let size = try #require(size), source = try #require(offline)
        let oversized = try injectedRaster(bytes: bytes)
        let reservation = try await scope.reserve(within: reserveBound)
        #expect(await reservation.load(source, size: size, loader: { _, _ in oversized }) == false)
        #expect(reservation.raster == nil)
        #expect(budget.active == 0)
        #expect(budget.charged == 0)
        #expect(budget.audit() == [])
        // The refused raster released the hold: the reservation can't load again.
        let calls = LoaderCalls()
        #expect(await reservation.load(source, size: size, loader: calls.loader(returning: oversized)) == false)
        #expect(calls.value == 0)
    }

    // MARK: Release clears the raster; close is terminal

    @Test func releasingAHoldClearsItsRasterOnScopeCancelAndClose() async throws {
        let window = try await blankWindow()
        defer { window.window.closeForTest() }
        let budget = PhotoBudget.inWindow(window.window)
        let detached = budget.makeScope(), mounted = budget.makeScope()
        defer { mounted.cancel() }
        let size = try #require(size), source = try #require(offline)
        let raster = try injectedRaster(bytes: mebibyte)
        let first = try await detached.reserve(within: reserveBound)
        let second = try await mounted.reserve(within: reserveBound)
        #expect(await first.load(source, size: size, loader: { _, _ in raster }))
        #expect(await second.load(source, size: size, loader: { _, _ in raster }))
        #expect(first.raster != nil && second.raster != nil)
        #expect(budget.charged == 2 * mebibyte)

        detached.cancel()
        #expect(first.raster == nil, "A detached image's raster must stop being drawn")
        #expect(second.raster?.bytes == mebibyte, "Another scope's raster must stay")
        #expect(budget.charged == mebibyte)

        window.window.closeForTest()
        #expect(second.raster == nil, "A closed window's raster must stop being drawn")
        #expect(budget.charged == 0)
        #expect(budget.audit() == [])
    }

    @Test func closingEndsTheBudgetAndAReopenedWindowGetsAFreshOne() async throws {
        let window = try await blankWindow()
        defer { window.window.closeForTest() }
        let budget = PhotoBudget.inWindow(window.window)
        let scope = budget.makeScope()
        defer { scope.cancel() }
        let held = try await scope.reserve(within: reserveBound)
        window.window.closeForTest()
        #expect(budget.closed)
        #expect(budget.active == 0 && budget.charged == 0 && budget.queued == 0)

        window.window.orderFrontRegardless()
        #expect(window.window.isVisible)
        // The closed budget admits nothing, from an old scope or a new one,
        // and has left the window table.
        #expect(await scope.reserve() == nil)
        #expect(await budget.makeScope().reserve() == nil)
        #expect(budget.queued == 0 && budget.active == 0 && budget.charged == 0)
        let fresh = PhotoBudget.inWindow(window.window)
        #expect(fresh !== budget)
        #expect(!fresh.closed)
        let freshScope = fresh.makeScope()
        defer { freshScope.cancel() }
        let reservation = try await freshScope.reserve(within: reserveBound)
        #expect(fresh.active == 1 && fresh.charged == reservedBytes)
        #expect(budget.active == 0 && budget.charged == 0)
        withExtendedLifetime((held, reservation)) {}
    }

    // MARK: A hidden window's turned-away work

    /// A scope that turns work away while its window is hidden stays not
    /// current when the window shows, so the probe replaces it and the image
    /// reruns. A scope that turned nothing away stays current.
    @Test func aScopeThatTurnsWorkAwayWhileItsWindowIsHiddenStaysNotCurrent() async throws {
        let window = try await blankWindow()
        defer { window.window.closeForTest() }
        let budget = PhotoBudget.inWindow(window.window)
        let size = try #require(size), source = try #require(offline)
        let raster = try injectedRaster(bytes: mebibyte)
        let loading = budget.makeScope(), waiting = budget.makeScope(), untouched = budget.makeScope()
        let refusing = budget.makeScope(), finishing = budget.makeScope()
        defer { for scope in [loading, waiting, untouched, refusing, finishing] { scope.cancel() } }
        var held: [PhotoBudget.Reservation] = []
        for _ in 0..<3 { held.append(try await loading.reserve(within: reserveBound)) }
        let gate = PhotoGate()
        let finished = try await finishing.reserve(within: reserveBound)
        let load = Task { await finished.load(source, size: size, loader: gate.loader) }
        try await waitForCollectionEvent("the gated load to begin") { gate.calls == 1 }
        let queued = Task { await waiting.reserve() }
        try await waitForCollectionEvent("one queued waiter") { budget.queued == 1 }
        let acquired = budget.active

        window.window.orderOut(nil)
        // A load that ends while hidden still holds its raster, but the scope
        // that reads it is not current, and stays so.
        gate.open(raster)
        let loaded = try await value(of: load, within: PhotoBudgetSequence.loadDrainBound, "loadDrainBound",
                                     "the opened load")
        #expect(loaded)
        #expect(!finishing.isCurrent)
        // Its freed slot wakes the waiter with nothing.
        let woken = try await value(of: queued, within: waiterBound, "waiterBound", "the woken waiter")
        #expect(woken == nil)
        #expect(budget.queued == 0)
        // Reserving refuses, and loading an acquired reservation refuses
        // without a request.
        #expect(await refusing.reserve() == nil)
        let calls = LoaderCalls()
        #expect(await held[0].load(source, size: size, loader: calls.loader(returning: raster)) == false)
        #expect(calls.value == 0)
        #expect(budget.active == acquired - 1)
        #expect(budget.audit() == [])

        window.window.orderFrontRegardless()
        #expect(window.window.isVisible)
        for scope in [finishing, waiting, refusing, loading] {
            #expect(!scope.isCurrent, "A scope that turned work away while hidden must stay not current")
        }
        #expect(untouched.isCurrent, "A scope that turned nothing away stays current")
        // The probe's replacement for a lapsed scope admits again.
        loading.cancel()
        #expect(budget.active == 0 && budget.charged == mebibyte)
        let fresh = budget.makeScope()
        defer { fresh.cancel() }
        let again = try await fresh.reserve(within: reserveBound)
        #expect(await again.load(source, size: size, loader: { _, _ in raster }))
        #expect(budget.audit() == [])
        withExtendedLifetime((held, finished)) {}
    }

    // MARK: The loader seam

    @Test func aReservationLoadsWithPhotoLoaderByDefault() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let window = try await blankWindow()
        defer { window.window.closeForTest() }
        let budget = PhotoBudget.inWindow(window.window)
        let scope = budget.makeScope()
        defer { scope.cancel() }
        let size = try #require(size)
        let reservation = try await scope.reserve(within: reserveBound)
        let viaReservation = try #require(PhotoSource(server.url("default-loader")))
        let viaLoader = try #require(PhotoSource(server.url("direct")))
        #expect(await reservation.load(viaReservation, size: size))
        // The default made the real request and decoded what PhotoLoader decodes.
        #expect(server.requests == ["/default-loader"])
        let direct = try #require(await PhotoLoader.load(viaLoader, size: size))
        let raster = try #require(reservation.raster)
        #expect(raster.bytes == direct.bytes)
        #expect(raster.first.width == direct.first.width && raster.first.height == direct.first.height)
        #expect(budget.charged == raster.bytes)
    }

    // MARK: Random sequences

    @Test func randomSequencesKeepTheBudgetConsistent() async throws {
        let environment = ProcessInfo.processInfo.environment
        let seeds: [UInt64] = environment["PHOTO_BUDGET_SEED"].flatMap { UInt64($0) }.map { [$0] }
            ?? [1, 2, 3, 5, 8, 13, 21, 34]
        let steps = environment["PHOTO_BUDGET_STEPS"].flatMap { Int($0) } ?? 400
        defer { PhotoBudgetAudit.context = "" }
        for seed in seeds {
            try await PhotoBudgetSequence(seed: seed, window: try await blankWindow()).run(steps: steps)
        }
    }
}
}

/// One seeded run of legal and illegal budget operations, checked after
/// every step against the audit, the literal limits and a model of each
/// handle's stage.
@MainActor private final class PhotoBudgetSequence {
    /// How long the test waits for a load task to end once its gate is open
    /// or its hold released. A load that is never woken fails here, by name,
    /// instead of hanging the run.
    static let loadDrainBound = Duration.seconds(5)

    private enum Expected {
        case acquired
        case loading(PhotoGate, Task<Bool, Never>)
        case holding(Int)
        case released
    }

    private final class Tracked {
        var reservation: PhotoBudget.Reservation?
        let scope: PhotoBudget.Scope
        var expected: Expected

        init(_ reservation: PhotoBudget.Reservation, scope: PhotoBudget.Scope, expected: Expected) {
            self.reservation = reservation
            self.scope = scope
            self.expected = expected
        }
    }

    @MainActor private final class LoadEnded {
        var value = false
    }

    /// The first of two racing results.
    @MainActor private final class Once {
        private var result: Bool??
        private var waiting: CheckedContinuation<Bool?, Never>?

        func resolve(_ value: Bool?) {
            guard result == nil else { return }
            result = .some(value)
            waiting?.resume(returning: value)
            waiting = nil
        }

        func value() async -> Bool? {
            if let result { return result }
            return await withCheckedContinuation { waiting = $0 }
        }
    }

    @MainActor private final class Arrivals {
        var values: [(PhotoBudget.Scope, PhotoBudget.Reservation?)] = []
    }

    private let seed: UInt64
    private var random: SplitMix64
    private let window: CollectionWindow
    private var budget: PhotoBudget
    private var closedBudgets: [PhotoBudget] = []
    private var scopes: [PhotoBudget.Scope]
    private var cancelledScopes: [PhotoBudget.Scope] = []
    private var tracked: [Tracked] = []
    private var reserves: [Task<Void, Never>] = []
    private var drains: [Task<Bool, Never>] = []
    private let arrivals = Arrivals()
    private var step = 0
    private var operation = ""
    private var holdingLoads = 0
    private var refusedFinishes = 0
    private var refusedLoads = 0
    private var peakActive = 0
    private var peakCharged = 0
    private var hidden = false
    /// Set when a load outlives `loadDrainBound`; the run stops at the end
    /// of that step rather than pile up one bound per stuck load.
    private var stuckLoad = false
    private var hides = 0
    private var lapsedScopes = 0
    /// Scopes that refused a load while the window was hidden. Each must
    /// still be not current once the window shows again.
    private var turnedAwayWhileHidden: [PhotoBudget.Scope] = []

    init(seed: UInt64, window: CollectionWindow) {
        self.seed = seed
        random = SplitMix64(state: seed)
        self.window = window
        budget = PhotoBudget.inWindow(window.window)
        scopes = (0..<3).map { _ in PhotoBudget.inWindow(window.window).makeScope() }
    }

    private var place: String { "seed \(seed) step \(step) (\(operation))" }

    func run(steps: Int) async throws {
        defer { window.window.closeForTest() }
        for index in 0..<steps {
            step = index
            try await perform()
            try await check()
            if stuckLoad {
                throw CollectionInputFailure(description: "\(place): stopped after a load outlived loadDrainBound")
            }
        }
        operation = "final close"
        PhotoBudgetAudit.context = place
        window.window.closeForTest()
        for task in reserves { task.cancel() }
        for task in reserves {
            try await value(of: task, within: waiterBound, "waiterBound", "\(place): a cancelled waiter")
        }
        var gates: [PhotoGate?] = drains.map { _ in nil }
        for item in tracked {
            if case .loading(let gate, let task) = item.expected {
                drains.append(task)
                gates.append(gate)
            }
        }
        for (task, gate) in zip(drains, gates) { _ = await drained(task, gate: gate, "the final drain") }
        collectArrivals()
        for budget in closedBudgets + [budget] {
            #expect(budget.audit() == [], "\(place)")
            #expect(budget.active == 0 && budget.charged == 0 && budget.queued == 0, "\(place)")
        }
        print("photo-budget seed \(seed): \(steps) steps, \(holdingLoads) holding loads, \(refusedFinishes) refused "
              + "finishes, \(refusedLoads) refused loads, \(closedBudgets.count) closes, peak active \(peakActive), "
              + "peak charged \(peakCharged), \(hides) hides, \(lapsedScopes) lapsed scopes replaced")
    }

    private func perform() async throws {
        let size = try #require(PhotoSize(points: CGSize(width: 64, height: 64), scale: 1))
        let source = try #require(PhotoSource("http://127.0.0.1:9/injected"))
        switch random.below(80) {
        case 0..<20:
            let useCancelled = !cancelledScopes.isEmpty && random.below(4) == 0
            let scope = useCancelled ? cancelledScopes[random.below(cancelledScopes.count)]
                : scopes[random.below(scopes.count)]
            operation = useCancelled ? "reserve on a cancelled scope" : "reserve"
            PhotoBudgetAudit.context = place
            let arrivals = arrivals
            reserves.append(Task { arrivals.values.append((scope, await scope.reserve())) })
        case 20..<23:
            operation = "cancel a pending reserve"
            PhotoBudgetAudit.context = place
            if !reserves.isEmpty { reserves[random.below(reserves.count)].cancel() }
        case 23..<41:
            // Mostly acquired handles, so loads begin; sometimes any handle,
            // so loads in every other stage are refused.
            let live = tracked.filter { $0.reservation != nil }
            let acquired = live.filter { if case .acquired = $0.expected { true } else { false } }
            let pool = !acquired.isEmpty && random.below(4) != 0 ? acquired : live
            guard !pool.isEmpty else { return }
            let item = pool[random.below(pool.count)]
            operation = "load in stage \(item.expected)"
            PhotoBudgetAudit.context = place
            try await load(item, source: source, size: size)
        case 41..<59:
            let loading = tracked.compactMap { item -> (Tracked, PhotoGate, Task<Bool, Never>)? in
                if case .loading(let gate, let task) = item.expected { (item, gate, task) } else { nil }
            }
            guard !loading.isEmpty else { return }
            let (item, gate, task) = loading[random.below(loading.count)]
            let choices: [Int?] = [nil, 1, 1 + random.below(reservedBytes), reservedBytes, reservedBytes + 1,
                                   64 * mebibyte]
            let bytes = choices[random.below(choices.count)]
            operation = "finish a load with \(bytes.map(String.init) ?? "nil")"
            PhotoBudgetAudit.context = place
            gate.open(try bytes.map { try injectedRaster(bytes: $0) })
            guard let loaded = await drained(task, gate: gate, "an opened load") else {
                item.expected = .released
                return
            }
            let fits = bytes.map { $0 <= reservedBytes } ?? false
            #expect(loaded == fits, "\(place)")
            if loaded { holdingLoads += 1 } else { refusedFinishes += 1 }
            item.expected = fits && item.reservation != nil ? .holding(bytes ?? 0) : .released
        case 59..<64:
            let live = tracked.filter { $0.reservation != nil }
            guard !live.isEmpty else { return }
            let item = live[random.below(live.count)]
            operation = "drop a handle in stage \(item.expected)"
            PhotoBudgetAudit.context = place
            item.reservation = nil
            // A loading handle stays alive in its load until the load ends.
            if case .loading = item.expected {} else { item.expected = .released }
        case 64..<68:
            let index = random.below(scopes.count)
            operation = "cancel scope \(index)"
            PhotoBudgetAudit.context = place
            let scope = scopes[index]
            scope.cancel()
            cancelledScopes.append(scope)
            scopes[index] = budget.makeScope()
            await releaseAll { $0.scope === scope }
        case 68:
            operation = "close and reopen"
            PhotoBudgetAudit.context = place
            window.window.closeForTest()
            #expect(budget.closed, "\(place)")
            await releaseAll { _ in true }
            closedBudgets.append(budget)
            window.window.orderFrontRegardless()
            hidden = false
            turnedAwayWhileHidden.removeAll()
            budget = PhotoBudget.inWindow(window.window)
            cancelledScopes.append(contentsOf: scopes)
            scopes = (0..<3).map { _ in budget.makeScope() }
        case 69, 70:
            if hidden {
                operation = "show the window"
                PhotoBudgetAudit.context = place
                try await show()
            } else {
                operation = "hide the window"
                PhotoBudgetAudit.context = place
                window.window.orderOut(nil)
                hidden = true
                hides += 1
            }
        default:
            operation = "yield"
            PhotoBudgetAudit.context = place
            for _ in 0..<8 { await Task.yield() }
        }
    }

    private func load(_ item: Tracked, source: PhotoSource, size: PhotoSize) async throws {
        guard let reservation = item.reservation else { return }
        let gate = PhotoGate()
        let ended = LoadEnded()
        let task = Task {
            let loaded = await reservation.load(source, size: size, loader: gate.loader)
            ended.value = true
            return loaded
        }
        if case .acquired = item.expected, !hidden {
            // The load begins and its loader starts waiting.
            let deadline = ContinuousClock.now + .seconds(5)
            while gate.calls == 0, ContinuousClock.now < deadline {
                try await Task.sleep(for: .milliseconds(1))
            }
            #expect(gate.calls == 1, "\(place): the load of an acquired reservation never started")
            item.expected = .loading(gate, task)
        } else {
            // Any other stage refuses at once and never calls the loader. A
            // load that wrongly begins is failed and opened, so it can't hang.
            refusedLoads += 1
            let deadline = ContinuousClock.now + .seconds(5)
            while !ended.value, gate.calls == 0, ContinuousClock.now < deadline {
                try await Task.sleep(for: .milliseconds(1))
            }
            #expect(gate.calls == 0, "\(place): a refused load called its loader")
            gate.open(nil)
            #expect(await drained(task, gate: gate, "a refused load") == false, "\(place)")
            // A hidden window refuses even an acquired reservation's load,
            // which stays acquired; its scope must stay not current.
            if hidden, case .acquired = item.expected { turnedAwayWhileHidden.append(item.scope) }
        }
    }

    /// Shows the window again and does what the image's viewport probe does
    /// on the refresh that follows: a scope that isn't current is cancelled
    /// and replaced, so its images rerun with a fresh one.
    private func show() async throws {
        window.window.orderFrontRegardless()
        hidden = false
        for scope in turnedAwayWhileHidden {
            #expect(!scope.isCurrent, "\(place): a scope that refused a load while hidden is current again")
        }
        turnedAwayWhileHidden.removeAll()
        for index in scopes.indices where !scopes[index].isCurrent {
            let scope = scopes[index]
            lapsedScopes += 1
            scope.cancel()
            cancelledScopes.append(scope)
            scopes[index] = budget.makeScope()
            await releaseAll { $0.scope === scope }
        }
        // A fresh scope admits while the window is visible.
        #expect(scopes.allSatisfy { $0.isCurrent }, "\(place): a fresh scope is not current")
    }

    /// The task's result, or nil after recording a named issue when it
    /// doesn't end within `loadDrainBound`. On a timeout the gate is opened
    /// with nil so the stuck load can unwind.
    private func drained(_ task: Task<Bool, Never>, gate: PhotoGate?, _ what: String) async -> Bool? {
        let result = Once()
        let waiting = Task { result.resolve(await task.value) }
        let timer = Task {
            try await Task.sleep(for: Self.loadDrainBound)
            result.resolve(nil)
        }
        let value = await result.value()
        timer.cancel()
        if value == nil {
            Issue.record("\(place): \(what) did not end within loadDrainBound (\(Self.loadDrainBound))")
            stuckLoad = true
            gate?.open(nil)
        }
        withExtendedLifetime(waiting) {}
        return value
    }

    /// Marks the matching handles released and waits for their cancelled loads.
    private func releaseAll(_ matches: (Tracked) -> Bool) async {
        for item in tracked where matches(item) {
            if case .loading(let gate, let task) = item.expected {
                let loaded = await drained(task, gate: gate, "a cancelled load")
                #expect(loaded != true, "\(place): a cancelled load returned a raster")
            }
            item.expected = .released
        }
    }

    private func collectArrivals() {
        for (scope, reservation) in arrivals.values {
            guard let reservation else { continue }
            // Closed budgets' scopes are in `cancelledScopes` too.
            let released = budget.closed || cancelledScopes.contains { $0 === scope }
            tracked.append(Tracked(reservation, scope: scope, expected: released ? .released : .acquired))
        }
        arrivals.values.removeAll()
    }

    private func check() async throws {
        for _ in 0..<4 { await Task.yield() }
        collectArrivals()
        for budget in closedBudgets + [budget] {
            let violations = budget.audit()
            #expect(violations == [], "\(place): \(violations)")
        }
        peakActive = max(peakActive, budget.active)
        peakCharged = max(peakCharged, budget.charged)
        #expect((0...slotLimit).contains(budget.active), "\(place): active=\(budget.active)")
        #expect((0...byteLimit).contains(budget.charged), "\(place): charged=\(budget.charged)")
        for item in tracked {
            guard let reservation = item.reservation else { continue }
            switch item.expected {
            case .holding(let bytes):
                #expect(reservation.raster?.bytes == bytes, "\(place): a holding handle lost its raster")
            case .acquired, .loading, .released:
                #expect(reservation.raster == nil, "\(place): a handle in stage \(item.expected) shows a raster")
            }
        }
        // Forget released handles the test no longer holds.
        tracked.removeAll { item in
            if item.reservation == nil, case .released = item.expected { return true }
            return false
        }
    }
}
