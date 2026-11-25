// CI PROBE ONLY (ci/ticket-mainrisk-appnap). Never merge to main.
// Runs the ContinuousClock refresh flow from theContinuousClockTimerRefreshesTheDrawing
// repeatedly, alternating between an App Nap opt-out (a user-initiated, latency-critical
// activity held for the iteration) and no opt-out, and prints when the timer was armed,
// when it fired, how many provider calls happened, and when the drawing updated.
// It records, it doesn't assert, so the run's other tests decide the job's result.
import AppKit
import Foundation
@testable import OpenUINative
import Synchronization
import Testing

@Suite(.serialized, .timeLimit(.minutes(5))) @MainActor struct TimerAppNapProbe {
    @Test func timerRefreshWithAndWithoutAppNapOptOut() async throws {
        print("[timer-probe] start appActive=\(NSApp?.isActive ?? false) lowPower=\(ProcessInfo.processInfo.isLowPowerModeEnabled) cpus=\(ProcessInfo.processInfo.activeProcessorCount)")
        for iteration in 1...8 {
            for optOut in [true, false] {
                try await probe(iteration: iteration, optOut: optOut)
            }
        }
    }

    private func probe(iteration: Int, optOut: Bool) async throws {
        let activity = optOut
            ? ProcessInfo.processInfo.beginActivity(options: [.userInitiated, .latencyCritical], reason: "timer probe")
            : nil
        defer { if let activity { ProcessInfo.processInfo.endActivity(activity) } }
        let clock = ContinuousClock()
        let start = clock.now
        func since(_ instant: ContinuousClock.Instant) -> String {
            let d = instant - start
            return String(format: "%.3f", Double(d.components.seconds) + Double(d.components.attoseconds) / 1e18)
        }
        let live = try await live("""
        q = Query("read", {}, "waiting", 1)
        root = TextContent(q)
        """)
        let callTimes = Mutex<[ContinuousClock.Instant]>([])
        try live.attachToolProvider { _, _ in
            let count = callTimes.withLock { times in
                times.append(ContinuousClock.now)
                return times.count
            }
            return .string(count == 1 ? "first" : "refreshed")
        }
        defer { try? live.detachToolProvider() }
        guard let firstTimer = live.queryDriver?.timer else {
            print("[timer-probe] i=\(iteration) optOut=\(optOut) NO TIMER ARMED")
            return
        }
        let armedAt = clock.now
        let fired = Task { @MainActor in
            await firstTimer.task.value
            return ContinuousClock.now
        }
        var firstDrawnAt: ContinuousClock.Instant?
        var refreshedAt: ContinuousClock.Instant?
        let limit = clock.now + .seconds(20)
        while refreshedAt == nil && clock.now < limit {
            let text = drawnText(live)
            if text == "first" && firstDrawnAt == nil { firstDrawnAt = clock.now }
            if text == "refreshed" { refreshedAt = clock.now }
            try await Task.sleep(for: .milliseconds(10))
        }
        let firedAt: ContinuousClock.Instant? = refreshedAt != nil ? await fired.value : nil
        fired.cancel()
        let calls = callTimes.withLock { $0 }
        print("[timer-probe] i=\(iteration) optOut=\(optOut) appActive=\(NSApp?.isActive ?? false) armedDeadline=\(firstTimer.deadline) armedAt=\(since(armedAt)) firstDrawnAt=\(firstDrawnAt.map(since) ?? "-") timerFiredAt=\(firedAt.map(since) ?? "not by 20s") calls=\(calls.count) callTimes=\(calls.map(since)) refreshedDrawnAt=\(refreshedAt.map(since) ?? "not by 20s") within5s=\(refreshedAt.map { $0 - armedAt <= .seconds(5) } ?? false)")
    }
}
