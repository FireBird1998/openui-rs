import Darwin
import Dispatch
import Foundation
import Synchronization

/// When a process exited, stamped with `ProcessInfo.systemUptime` by a
/// dispatch process source's exit handler on a global queue. The stamp is
/// taken off the main actor, so a test that checks it later, after a late
/// main-actor wake, still sees when the process exited. If the process has
/// already gone when the source is installed, dispatch reports the exit at
/// once, so the stamp is the install time: never earlier than the exit.
final class ExitStamp: Sendable {
    enum Outcome: Sendable {
        /// The handler's stamp.
        case exited(at: TimeInterval)
        /// A check made at or after the deadline found the process running;
        /// the time of that check.
        case runningAtDeadline(at: TimeInterval)
        /// A check made at or after the deadline found the process exited,
        /// and still no stamp had come a recheck later: the source watched a
        /// reused pid, or no exit event came. The time of the recheck.
        case exitedWithoutStamp(at: TimeInterval)
    }

    /// How long after a check that finds the process exited the stamp may
    /// still come before the outcome is `exitedWithoutStamp`. Its length
    /// cannot fail an exit that was in time: the check ran at or after the
    /// deadline, so any stamp taken after it is at or after the deadline too.
    /// It only names the failure before the watchdog would.
    static let stampRecheck: TimeInterval = 1

    private struct State {
        var outcome: Outcome?
        var exitedAt: TimeInterval?
        var waiter: CheckedContinuation<Outcome, Never>?
    }

    let pid: pid_t
    private let state = Mutex(State())
    private let queue = DispatchQueue(label: "org.openui.tests.exit-stamp", qos: .userInitiated)
    private let source: any DispatchSourceProcess

    init(_ pid: pid_t) {
        self.pid = pid
        source = DispatchSource.makeProcessSource(identifier: pid, eventMask: .exit, queue: queue)
        source.setEventHandler { [weak self] in self?.stampExit() }
        source.activate()
    }

    deinit { source.cancel() }

    private func stampExit() {
        let now = ProcessInfo.processInfo.systemUptime
        source.cancel()
        resolve { state in
            guard state.exitedAt == nil else { return nil }
            state.exitedAt = now
            return .exited(at: now)
        }
    }

    /// Settles the outcome once and wakes the waiter.
    private func resolve(_ decide: (inout State) -> Outcome?) {
        let woken: (CheckedContinuation<Outcome, Never>, Outcome)? = state.withLock { state in
            guard state.outcome == nil, let outcome = decide(&state) else { return nil }
            state.outcome = outcome
            defer { state.waiter = nil }
            return state.waiter.map { ($0, outcome) }
        }
        if let (waiter, outcome) = woken { waiter.resume(returning: outcome) }
    }

    /// The exit stamp, or, if `exited` is false at a check on this queue at
    /// or after `deadline`, that check's time. A check that runs late never
    /// invents a failure: it reports "running" only for a process running
    /// past the deadline. A process that has exited by then waits for its
    /// stamp, which says when, for up to `stampRecheck`; then the outcome is
    /// `exitedWithoutStamp`.
    func outcome(by deadline: TimeInterval, exited: @escaping @Sendable () -> Bool) async -> Outcome {
        scheduleCheck(at: deadline, exited: exited)
        return await withCheckedContinuation { (waiter: CheckedContinuation<Outcome, Never>) in
            let settled: Outcome? = state.withLock { state in
                if let outcome = state.outcome { return outcome }
                state.waiter = waiter
                return nil
            }
            if let settled { waiter.resume(returning: settled) }
        }
    }

    private func scheduleCheck(at deadline: TimeInterval, exited: @escaping @Sendable () -> Bool) {
        let wait = max(0, deadline - ProcessInfo.processInfo.systemUptime)
        queue.asyncAfter(deadline: .now() + wait) { [weak self] in
            guard let self else { return }
            let now = ProcessInfo.processInfo.systemUptime
            // Dispatch time and uptime are separate clocks; never decide early.
            guard now >= deadline else { return self.scheduleCheck(at: deadline, exited: exited) }
            guard !exited() else { return self.recheckForStamp() }
            self.resolve { _ in .runningAtDeadline(at: now) }
        }
    }

    /// The exit handler runs on this queue too; if it has not stamped by the
    /// recheck, say so instead of waiting for the watchdog.
    private func recheckForStamp() {
        queue.asyncAfter(deadline: .now() + Self.stampRecheck) { [weak self] in
            guard let self else { return }
            let now = ProcessInfo.processInfo.systemUptime
            self.resolve { _ in .exitedWithoutStamp(at: now) }
        }
    }
}

/// Watches the directory holding a helper's report off the main actor and,
/// the moment the report appears and names a process, starts that process's
/// `ExitStamp`. The helper writes its report just before it exits, and its
/// WebContent exits soon after, so the watch must not wait for the main
/// actor to see the helper's exit first.
final class HostExitReportWatch: Sendable {
    private let report: URL
    private let pid: @Sendable (Data) -> pid_t?
    private let stamped = Mutex<ExitStamp?>(nil)
    private let queue = DispatchQueue(label: "org.openui.tests.host-exit-report", qos: .userInitiated)
    private let source: any DispatchSourceFileSystemObject

    struct OpenFailure: Error, CustomStringConvertible {
        let description: String
    }

    init(_ report: URL, pid: @escaping @Sendable (Data) -> pid_t?) throws {
        self.report = report
        self.pid = pid
        let directory = report.deletingLastPathComponent()
        let descriptor = open(directory.path, O_EVTONLY)
        guard descriptor >= 0 else {
            throw OpenFailure(description: "Could not watch \(directory.path): errno \(errno)")
        }
        // The atomic write renames the report into the directory: a write event.
        source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: descriptor, eventMask: .write, queue: queue)
        source.setEventHandler { [weak self] in self?.check() }
        source.setCancelHandler { close(descriptor) }
        source.activate()
    }

    deinit { source.cancel() }

    private func check() {
        guard stamped.withLock({ $0 == nil }),
              let data = try? Data(contentsOf: report), let pid = pid(data) else { return }
        let stamp = ExitStamp(pid)
        stamped.withLock { $0 = stamp }
        source.cancel()
    }

    /// The exit stamp the report started. If no event has started one yet,
    /// read the report now (a later, fallback start).
    func stamp() -> ExitStamp? {
        if let stamp = stamped.withLock({ $0 }) { return stamp }
        queue.sync { check() }
        return stamped.withLock { $0 }
    }

    func cancel() { source.cancel() }
}
