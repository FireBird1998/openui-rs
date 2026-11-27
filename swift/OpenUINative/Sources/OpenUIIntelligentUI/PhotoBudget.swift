import AppKit
import Observation
import os

/// Every image in the hosting window shares this allowance. A host that
/// draws several programs in one window shares the same limits across them.
///
/// A reservation's life is one state machine. Its stage is written only by
/// the five transitions `admit`, `begin`, `finish`, `release` and
/// `releaseAll`, which are also the only code that changes `active` (through
/// the `Slot` token they create and drop) and `charged`. A transition whose
/// starting stage doesn't match refuses and changes nothing.
@MainActor final class PhotoBudget: NSObject {
    private static let windows = NSMapTable<NSWindow, PhotoBudget>.weakToStrongObjects()

    private weak var window: NSWindow?
    /// Set when the window closes. A closed budget has left `windows`, holds
    /// nothing and admits nothing; a reopened window gets a fresh budget.
    private(set) var closed = false
    private(set) var active = 0
    private(set) var charged = 0

    private struct Waiter {
        let id: UUID
        let scope: Scope
        let continuation: CheckedContinuation<Reservation?, Never>
    }

    /// The budget's record of one reservation. A hold returns its slot and
    /// bytes only by leaving `holds`, and it can leave only once. The slot
    /// lives in the stages that may download and the raster in the stage that
    /// draws, so a holding reservation can't keep a slot.
    private struct Hold {
        enum Stage {
            case acquired(Slot)
            case loading(Slot, Task<PhotoRaster?, Never>)
            case holding(PhotoRaster)
        }   // Released: absent from `holds`.

        let scopeID: UUID
        /// The charge taken at admission, the most this hold may ever charge.
        let reserved: Int
        var stage: Stage
        weak var handle: Reservation?

        var bytes: Int {
            if case .holding(let raster) = stage { raster.bytes } else { reserved }
        }
    }

    enum ReleaseReason {
        /// The caller dropped its handle.
        case dropped
        /// The load returned no raster.
        case failed
        /// The load returned a raster larger than the reservation.
        case oversized
        /// The image's viewport probe cancelled its scope.
        case scope
        /// The window closed.
        case closed
    }

    private var waiters: [Waiter] = []
    private var holds: [UUID: Hold] = [:]
    var queued: Int { waiters.count }

    static func inWindow(_ window: NSWindow) -> PhotoBudget {
        if let existing = windows.object(forKey: window) { return existing }
        let budget = PhotoBudget(window: window)
        windows.setObject(budget, forKey: window)
        return budget
    }

    private init(window: NSWindow) {
        self.window = window
        super.init()
        NotificationCenter.default.addObserver(self, selector: #selector(windowWillClose),
                                               name: NSWindow.willCloseNotification, object: window)
    }

    @objc private func windowWillClose(_ notification: Notification) {
        // The notification precedes the visibility change. Close the budget
        // before any connection can unwind and wake the queue.
        guard !closed else { return }
        closed = true
        NotificationCenter.default.removeObserver(self)
        if let window, Self.windows.object(forKey: window) === self {
            Self.windows.removeObject(forKey: window)
        }
        releaseAll(scope: nil, .closed)
    }

    func makeScope() -> Scope { Scope(budget: self) }

    /// One attachment of the image's viewport probe to a window.
    @MainActor final class Scope {
        fileprivate let budget: PhotoBudget
        fileprivate let id = UUID()
        private var cancelled = false
        /// Set when the scope is found not current only because its window
        /// is invisible. It never clears.
        private var lapsed = false

        fileprivate init(budget: PhotoBudget) {
            self.budget = budget
        }

        func belongs(to window: NSWindow?) -> Bool { budget.window === window }

        /// Whether the scope may still admit, load and show. A scope found
        /// not current while its window is invisible stays not current. Every
        /// reader turns work away when this is false (a queued waiter woken
        /// with nil, a load refused, a finished load not shown), so the probe
        /// must make a fresh scope, and the image rerun its task, once the
        /// window is visible again.
        var isCurrent: Bool {
            guard !cancelled, !lapsed, !budget.closed else { return false }
            guard budget.window?.isVisible == true else {
                lapsed = true
                return false
            }
            return true
        }

        func reserve() async -> Reservation? { await budget.reserve(in: self) }

        func cancel() {
            guard !cancelled else { return }
            cancelled = true
            budget.releaseAll(scope: id, .scope)
        }
    }

    private func reserve(in scope: Scope) async -> Reservation? {
        let id = UUID()
        return await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                guard scope.isCurrent, !Task.isCancelled, waiters.count < PhotoLimits.queuedLoads else {
                    continuation.resume(returning: nil)
                    return
                }
                waiters.append(Waiter(id: id, scope: scope, continuation: continuation))
                admitWaitingLoads()
            }
        } onCancel: {
            Task { @MainActor in self.cancelWaiter(id) }
        }
    }

    private func cancelWaiter(_ id: UUID) {
        if let index = waiters.firstIndex(where: { $0.id == id }) {
            waiters.remove(at: index).continuation.resume(returning: nil)
        }
    }

    private func admitWaitingLoads() {
        // Each pass drops an invalid waiter or admits one. Queue insertion
        // bounds the total work of this scan.
        for _ in 0..<PhotoLimits.queuedLoads {
            guard let next = waiters.first else { return }
            guard next.scope.isCurrent else {
                waiters.removeFirst()
                next.continuation.resume(returning: nil)
                continue
            }
            guard let reservation = admit(next) else { return }
            waiters.removeFirst()
            next.continuation.resume(returning: reservation)
        }
    }

    // MARK: Transitions

    /// Queued → Acquired. Takes a slot and the maximum raster charge and
    /// registers them with the waiter's scope in one step, so closing the
    /// window or cancelling the scope finds the reservation from the moment
    /// it exists. Nil, changing nothing, when there is no room.
    private func admit(_ waiter: Waiter) -> Reservation? {
        transition {
            let reserved = PhotoLimits.decodedImageBytes
            guard !closed, holds[waiter.id] == nil, active < PhotoLimits.concurrentLoads,
                  charged + reserved <= PhotoLimits.windowRasterBytes else { return nil }
            let reservation = Reservation(scope: waiter.scope, id: waiter.id)
            holds[waiter.id] = Hold(scopeID: waiter.scope.id, reserved: reserved,
                                    stage: .acquired(Slot(budget: self)), handle: reservation)
            charged += reserved
            return reservation
        }
    }

    /// Acquired → Loading: starts the hold's one download with its slot. Nil,
    /// without calling `start`, in any other stage: a slot serves one load.
    private func begin(_ id: UUID, _ start: () -> Task<PhotoRaster?, Never>) -> Task<PhotoRaster?, Never>? {
        transition {
            guard var hold = holds[id], case .acquired(let slot) = hold.stage else { return nil }
            let task = start()
            hold.stage = .loading(slot, task)
            holds[id] = hold
            return task
        }
    }

    /// Loading → Holding when the raster fits the reservation: the slot is
    /// spent and the charge becomes the raster's actual bytes, which never
    /// grows it. Loading → Released for no raster or one larger than the
    /// reservation. False, changing nothing, in any other stage.
    private func finish(_ id: UUID, _ raster: PhotoRaster?) -> Bool {
        transition {
            guard let hold = holds[id], case .loading(let slot, _) = hold.stage else { return false }
            guard let raster else {
                release(id, .failed)
                return false
            }
            guard raster.bytes <= hold.reserved else {
                release(id, .oversized)
                return false
            }
            var next = hold
            next.stage = .holding(raster)
            holds[id] = next
            charged -= hold.reserved - raster.bytes
            next.handle?.raster = raster
            // Keep the token alive past the dictionary write so its deinit,
            // which retries admission, runs outside that access.
            withExtendedLifetime(slot) {}
            return true
        }
    }

    /// Any live stage → Released: returns the slot and bytes the hold still
    /// has, cancels its download and clears the handle's raster. Does nothing
    /// for a hold that was already released.
    private func release(_ id: UUID, _ reason: ReleaseReason) {
        transition {
            // Removal is the only way out of `holds`, so this runs once per hold.
            guard let hold = holds.removeValue(forKey: id) else { return }
            let hadSlot: Bool
            switch hold.stage {
            case .acquired:
                hadSlot = true
            case .loading(_, let task):
                hadSlot = true
                task.cancel()
            case .holding:
                hadSlot = false
            }
            charged -= hold.bytes
            if let handle = hold.handle {
                handle.raster = nil
                #if DEBUG
                releasedHandles.append(WeakReservation(handle))
                #endif
            }
            // The removed hold owns the slot token. Dropping it after the
            // dictionary access returns the slot and retries admission.
            if !hadSlot { admitWaitingLoads() }
        }
    }

    /// Releases every hold and drops every waiter in the scope, or in the
    /// whole window for nil, whatever stage each reservation has reached.
    private func releaseAll(scope scopeID: UUID?, _ reason: ReleaseReason) {
        transition {
            let dropped = waiters.filter { scopeID == nil || $0.scope.id == scopeID }
            waiters.removeAll { scopeID == nil || $0.scope.id == scopeID }
            for waiter in dropped { waiter.continuation.resume(returning: nil) }
            let released = holds.compactMap { scopeID == nil || $0.value.scopeID == scopeID ? $0.key : nil }
            for id in released { release(id, reason) }
        }
    }

    /// The hold is the token's only owner. Its only slot return is destruction
    /// of this token, which happens once when the hold gives it up.
    @MainActor private final class Slot {
        private let budget: PhotoBudget

        init(budget: PhotoBudget) {
            self.budget = budget
            budget.active += 1
        }

        isolated deinit {
            budget.active -= 1
            budget.admitWaitingLoads()
        }
    }

    /// The caller's handle to one hold. Dropping it releases whatever the hold
    /// still has; the budget may have released it already.
    @MainActor @Observable final class Reservation {
        private let scope: Scope
        fileprivate let id: UUID

        /// The loaded raster while the hold is Holding, nil in every other
        /// stage. Set only by `finish` and cleared by `release`, so a released
        /// hold stops being drawn.
        fileprivate(set) var raster: PhotoRaster?

        private var budget: PhotoBudget { scope.budget }

        fileprivate init(scope: Scope, id: UUID) {
            self.scope = scope
            self.id = id
        }

        /// Spends the reservation's slot on one load. Returns false without a
        /// request when the slot was already spent or released, and false when
        /// the load fails. On true, `raster` holds the result.
        ///
        /// `loader` is an internal test seam, not API: production callers use
        /// the default, `PhotoLoader.load`, and tests inject rasters with it.
        func load(_ source: PhotoSource, size: PhotoSize,
                  loader: @escaping @Sendable (PhotoSource, PhotoSize) async -> PhotoRaster? = PhotoLoader.load)
            async -> Bool {
            guard scope.isCurrent, !Task.isCancelled,
                  let task = budget.begin(id, { Task { await loader(source, size) } })
            else { return false }
            let raster = await withTaskCancellationHandler {
                await task.value
            } onCancel: {
                task.cancel()
            }
            return budget.finish(id, raster)
        }

        isolated deinit {
            budget.release(id, .dropped)
        }
    }

    // MARK: Invariants

    /// The budget's state in plain values, for `audit`.
    struct Snapshot {
        enum Stage: Equatable {
            case acquired
            case loading
            case holding(bytes: Int)
        }

        struct Hold {
            var stage: Stage
            var reserved: Int
            /// The bytes of the raster the hold's handle shows, nil for none
            /// or for a handle that's gone.
            var handleRasterBytes: Int?
        }

        var closed: Bool
        var active: Int
        var charged: Int
        var waiters: Int
        var holds: [Hold]
        /// Handles released in this transition that still show a raster.
        var releasedHandlesWithRaster: Int
    }

    /// The violated invariants of a snapshot, empty when it is consistent.
    static func audit(_ state: Snapshot) -> [String] {
        var violations: [String] = []
        var sum = 0
        var withSlot = 0
        for (index, hold) in state.holds.enumerated() {
            let bytes: Int
            switch hold.stage {
            case .acquired, .loading:
                bytes = hold.reserved
                withSlot += 1
                if hold.handleRasterBytes != nil {
                    violations.append("hold \(index) shows a raster before it loaded")
                }
            case .holding(let rasterBytes):
                bytes = rasterBytes
                if let shown = hold.handleRasterBytes, shown != rasterBytes {
                    violations.append("hold \(index) shows \(shown) bytes but charges \(rasterBytes)")
                }
            }
            if bytes > hold.reserved {
                violations.append("hold \(index) charges \(bytes) bytes, over its reserved \(hold.reserved)")
            }
            sum += bytes
        }
        if state.charged != sum {
            violations.append("charged \(state.charged) is not the holds' sum \(sum)")
        }
        if state.charged < 0 || state.charged > PhotoLimits.windowRasterBytes {
            violations.append("charged \(state.charged) is outside 0...\(PhotoLimits.windowRasterBytes)")
        }
        if state.active != withSlot {
            violations.append("active \(state.active) is not the \(withSlot) acquired or loading holds")
        }
        if state.active > PhotoLimits.concurrentLoads {
            violations.append("active \(state.active) is over \(PhotoLimits.concurrentLoads)")
        }
        if state.closed, !state.holds.isEmpty || state.waiters > 0 {
            violations.append("closed budget keeps \(state.holds.count) holds and \(state.waiters) waiters")
        }
        if state.releasedHandlesWithRaster > 0 {
            violations.append("\(state.releasedHandlesWithRaster) released handles still show a raster")
        }
        return violations
    }

    func audit() -> [String] { Self.audit(snapshot()) }

    private func snapshot() -> Snapshot {
        let holds = holds.values.map { hold -> Snapshot.Hold in
            let stage: Snapshot.Stage = switch hold.stage {
            case .acquired: .acquired
            case .loading: .loading
            case .holding(let raster): .holding(bytes: raster.bytes)
            }
            return Snapshot.Hold(stage: stage, reserved: hold.reserved, handleRasterBytes: hold.handle?.raster?.bytes)
        }
        let releasedWithRaster = releasedHandles.count(where: { $0.reservation?.raster != nil })
        return Snapshot(closed: closed, active: active, charged: charged, waiters: waiters.count, holds: holds,
                        releasedHandlesWithRaster: releasedWithRaster)
    }

    /// Called with the violations when an audit fails. Tests install it and
    /// record an issue. With none installed, the first violation is logged.
    static var onViolation: (([String]) -> Void)?
    private static var loggedViolation = false
    private static let log = Logger(subsystem: "OpenUIIntelligentUI", category: "PhotoBudget")

    private struct WeakReservation {
        weak var reservation: Reservation?
        init(_ reservation: Reservation) { self.reservation = reservation }
    }

    private var depth = 0
    private var releasedHandles: [WeakReservation] = []

    /// Runs one transition. Nested transitions (a release that admits a
    /// waiter, a scope cancel that releases each hold) are audited once, when
    /// the outermost one returns, so the audit never sees a half-done step.
    /// Debug builds only: the audit is O(holds), bounded by mounted images.
    private func transition<T>(_ body: () -> T) -> T {
        depth += 1
        defer {
            depth -= 1
            if depth == 0 {
                #if DEBUG
                reportViolations(audit())
                #endif
                releasedHandles.removeAll()
            }
        }
        return body()
    }

    private func reportViolations(_ violations: [String]) {
        guard !violations.isEmpty else { return }
        if let onViolation = Self.onViolation {
            onViolation(violations)
            return
        }
        // This deliberately doesn't assert. It's a library, and an assertion
        // would stop a developer's app over our own bookkeeping. Log the
        // first violation once and carry on: the limits still bound the work.
        guard !Self.loggedViolation else { return }
        Self.loggedViolation = true
        Self.log.fault("Photo budget invariant violated: \(violations.joined(separator: "; "), privacy: .public)")
    }
}
