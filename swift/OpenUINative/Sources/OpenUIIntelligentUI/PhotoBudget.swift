import AppKit

/// Every image in the hosting window shares this allowance. A host that
/// draws several programs in one window shares the same limits across them.
@MainActor final class PhotoBudget {
    private static let windows = NSMapTable<NSWindow, PhotoBudget>.weakToStrongObjects()

    private var active = 0
    private(set) var charged = 0
    private struct Waiter {
        let id: UUID
        let continuation: CheckedContinuation<Reservation?, Never>
    }
    private var waiters: [Waiter] = []

    static func inWindow(_ window: NSWindow) -> PhotoBudget {
        if let existing = windows.object(forKey: window) { return existing }
        let budget = PhotoBudget()
        windows.setObject(budget, forKey: window)
        return budget
    }

    func reserve() async -> Reservation? {
        let id = UUID()
        return await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                guard !Task.isCancelled, waiters.count < PhotoLimits.queuedLoads else {
                    continuation.resume(returning: nil)
                    return
                }
                waiters.append(Waiter(id: id, continuation: continuation))
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
        for _ in 0..<PhotoLimits.concurrentLoads {
            guard active < PhotoLimits.concurrentLoads,
                  charged <= PhotoLimits.windowRasterBytes - PhotoLimits.decodedImageBytes,
                  !waiters.isEmpty else { return }
            active += 1
            charged += PhotoLimits.decodedImageBytes
            waiters.removeFirst().continuation.resume(returning: Reservation(budget: self))
        }
    }

    @MainActor final class Reservation {
        private let budget: PhotoBudget
        private var bytes = PhotoLimits.decodedImageBytes
        private var downloading = true

        fileprivate init(budget: PhotoBudget) { self.budget = budget }

        func retain(_ raster: PhotoRaster) {
            guard downloading else { return }
            downloading = false
            budget.active -= 1
            budget.charged -= bytes - raster.bytes
            bytes = raster.bytes
            budget.admitWaitingLoads()
        }

        isolated deinit {
            if downloading { budget.active -= 1 }
            budget.charged -= bytes
            budget.admitWaitingLoads()
        }
    }
}
