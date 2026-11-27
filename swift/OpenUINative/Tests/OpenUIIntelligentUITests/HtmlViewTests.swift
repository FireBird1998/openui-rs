import AppKit
import Darwin
import Foundation
import OpenUINative
@testable import OpenUIIntelligentUI
import SwiftUI
import SwiftTestSupport
import Testing
import WebKit

extension CollectionDrawingTests {
@Suite @MainActor struct HtmlViewTests {
    private let diagnostics = BrowserDiagnostics()

    // The readiness wait's bound, proved at a scaled 1 s. The fixture ships
    // `HtmlViewLoopback.readinessBound` (60 s), checked here too.
    @Test(.timeLimit(.minutes(1)), NativeTestWatchdog()) func fixtureThatNeverSignalsFailsAtTheReadinessBound() async throws {
        #expect(HtmlViewLoopback.readinessBound == .seconds(60))
        let (child, readiness) = try launchReadinessChild("exec /bin/sleep 600")
        defer { if child.isRunning { kill(child.processIdentifier, SIGKILL) } }
        let clock = ContinuousClock()
        let started = clock.now
        let failure = await readinessFailure(of: child, on: readiness, within: .seconds(1))
        let elapsed = clock.now - started
        #expect(failure?.contains("the HtmlView fixture didn't signal readiness within 1 s; PID \(child.processIdentifier) was still running at the bound and was killed.") == true,
                "\(String(describing: failure))")
        #expect(elapsed >= .seconds(1) && elapsed < .seconds(5), "Failed after \(elapsed)")
        #expect(!child.isRunning, "The silent child was killed at the bound")
    }

    @Test(.timeLimit(.minutes(1)), NativeTestWatchdog()) func fixtureThatExitsBeforeSignallingFailsAtOnce() async throws {
        let (child, readiness) = try launchReadinessChild("exit 3")
        defer { if child.isRunning { kill(child.processIdentifier, SIGKILL) } }
        let clock = ContinuousClock()
        let started = clock.now
        let failure = await readinessFailure(of: child, on: readiness, within: HtmlViewLoopback.readinessBound)
        let elapsed = clock.now - started
        #expect(failure?.contains("PID \(child.processIdentifier) exited with status 3 before signalling.") == true,
                "\(String(describing: failure))")
        #expect(elapsed < .seconds(5), "Failed after \(elapsed), not at once")
    }

    @Test(.timeLimit(.minutes(1)), NativeTestWatchdog()) func fixtureThatSignalsIsReadyBeforeTheBound() async throws {
        let (child, readiness) = try launchReadinessChild("printf R; exec /bin/sleep 600")
        defer { if child.isRunning { kill(child.processIdentifier, SIGKILL) } }
        let failure = await readinessFailure(of: child, on: readiness, within: HtmlViewLoopback.readinessBound)
        #expect(failure == nil, "\(String(describing: failure))")
        #expect(child.isRunning)
    }

    private func launchReadinessChild(_ shell: String) throws -> (Process, Pipe) {
        let child = Process()
        child.executableURL = URL(fileURLWithPath: "/bin/sh")
        child.arguments = ["-c", shell]
        let readiness = Pipe()
        child.standardOutput = readiness
        try child.run()
        return (child, readiness)
    }

    private func readinessFailure(of child: Process, on readiness: Pipe, within bound: Duration) async -> String? {
        do {
            try await HtmlViewLoopback.awaitReadiness(of: child, on: readiness, within: bound)
            return nil
        } catch {
            return String(describing: error)
        }
    }

    @Test(.requiresDisplay, NativeTestWatchdog()) func inlineScriptRunsAndRequestsOutsidePolicyAreBlocked() async throws {
        let server = try await HtmlViewLoopback.start()
        defer { server.stop() }
        let mounted = try await mount("""
        <body>
        <script>
        document.body.append('inline script ran');
        addEventListener('securitypolicyviolation', event => {
            if (event.disposition !== 'enforce' ||
                !event.blockedURI.startsWith('http://127.0.0.1')) return;
            if (event.effectiveDirective === 'connect-src')
                document.body.append('connection refused by policy');
            if (event.effectiveDirective.startsWith('script-src'))
                document.body.append('external script refused by policy');
        });
        fetch('\(server.http.appending(path: "data").absoluteString)').catch(() => {});
        fetch('/host-resource').catch(() => document.body.append('relative host resource refused'));
        const image = new Image();
        image.onerror = () => document.body.append('loopback image refused');
        image.src = '\(server.http.appending(path: "image.svg").absoluteString)';
        document.body.append(image);
        const script = document.createElement('script');
        script.src = '\(server.http.appending(path: "script.js").absoluteString)';
        document.head.append(script);
        const file = document.createElement('script');
        file.onerror = () => document.body.append('file script refused');
        file.src = \(try literal(server.fileProbe.absoluteString));
        document.head.append(file);
        try { eval("document.body.append('eval was allowed')"); }
        catch (error) {
            if (error instanceof EvalError) document.body.append('eval refused by policy');
        }
        let parentReadable = false;
        try { parentReadable = !!parent.document; } catch {}
        if (!parentReadable && !window.webkit?.messageHandlers?.htmlViewHeight)
            document.body.append('page cannot reach host');
        </script>
        """)
        defer { mounted.window.closeForTest() }
        let web = try await webView(in: mounted)
        try await requireText("inline script ran", in: web)
        try await requireText("connection refused by policy", in: web)
        try await requireText("relative host resource refused", in: web)
        try await requireText("loopback image refused", in: web)
        try await requireText("external script refused by policy", in: web)
        try await requireText("file script refused", in: web)
        try await requireText("eval refused by policy", in: web)
        try await requireText("page cannot reach host", in: web)
        #expect(server.requests.isEmpty)
    }

    @Test(.requiresDisplay, NativeTestWatchdog()) func storageIsUnavailableInTwoViewsAndAFreshSession() async throws {
        let writer = storageProbe(write: true, marker: "writer denied all storage")
        let reader = storageProbe(write: false, marker: "reader found no storage")
        do {
            let mounted = try await CollectionWindow("""
            root = Col([HtmlView(\(try literal(writer)), 80), HtmlView(\(try literal(reader)), 80)], 0)
            """)
            defer { mounted.window.closeForTest() }
            try await waitFor("both storage probes mounted", in: mounted) { webViews(in: mounted.host).count == 2 }
            let views = webViews(in: mounted.host)
            try #require(views.count == 2)
            let first = try #require(views.first)
            let second = try #require(views.last)
            #expect(!first.configuration.websiteDataStore.isPersistent)
            #expect(!second.configuration.websiteDataStore.isPersistent)
            #expect(first.configuration.websiteDataStore !== second.configuration.websiteDataStore)
            let writerView = try await requireText("writer denied all storage", in: first, second)
            let readerView = try await requireText("reader found no storage", in: first, second)
            #expect(writerView !== readerView)
        }
        let relaunched = try await mount(reader)
        defer { relaunched.window.closeForTest() }
        let fresh = try await webView(in: relaunched)
        #expect(!fresh.configuration.websiteDataStore.isPersistent)
        try await requireText("reader found no storage", in: fresh)
    }

    private func storageProbe(write: Bool, marker: String) -> String {
        """
        <body><script>
        const write = \(write);
        const key = 'openui_html_test';
        async function available(probe) {
            try { return await probe(); } catch { return false; }
        }
        Promise.all([
            available(() => {
                if (write) document.cookie = key + '=first';
                return document.cookie.includes(key + '=');
            }),
            available(() => {
                if (write) localStorage.setItem(key, 'first');
                return localStorage.getItem(key) !== null;
            }),
            available(() => {
                if (write) sessionStorage.setItem(key, 'first');
                return sessionStorage.getItem(key) !== null;
            }),
            available(() => write
                ? new Promise(resolve => {
                    const request = indexedDB.open(key);
                    request.onsuccess = () => { request.result.close(); resolve(true); };
                    request.onerror = () => resolve(false);
                })
                : indexedDB.databases().then(databases => databases.some(db => db.name === key))),
            available(async () => {
                if (write) await caches.open(key);
                return (await caches.keys()).includes(key);
            })
        ]).then(results => {
            document.body.textContent = origin === "null" && !results.some(Boolean)
                ? "\(marker)" : "browser storage was available";
        });
        </script>
        """
    }

    @Test(.requiresDisplay, .timeLimit(.minutes(2)))
    func httpCacheIsSeparateAcrossViewsAndAppProcesses() async throws {
        let server = try await HtmlViewLoopback.start()
        defer { server.stop() }
        let page = cacheProbe(server, marker: "first")
        let second = cacheProbe(server, marker: "second")
        do {
            let mounted = try await CollectionWindow("""
            root = Col([HtmlView(\(try literal(page)), 80), HtmlView(\(try literal(second)), 80)], 0)
            """)
            defer { mounted.window.closeForTest() }
            try await waitFor("both cache probes mounted", in: mounted) { webViews(in: mounted.host).count == 2 }
            let views = webViews(in: mounted.host)
            let firstView = try #require(views.first)
            let secondView = try #require(views.last)
            try await loadCacheProbe(server, marker: "first", in: firstView, mounted: mounted, loads: 2)
            try #require(server.requests == ["/cache.svg"], "Reusing a fresh cached resource sends no second request")
            try await loadCacheProbe(server, marker: "second", in: secondView, mounted: mounted)
            try #require(server.requests == ["/cache.svg", "/cache.svg"])
        }

        do {
            let fresh = try await mount(cacheProbe(server, marker: "fresh"), height: 80)
            defer { fresh.window.closeForTest() }
            let freshView = try await webView(in: fresh)
            try await loadCacheProbe(server, marker: "fresh", in: freshView, mounted: fresh)
            try #require(server.requests == Array(repeating: "/cache.svg", count: 3))
        }

        let directory = server.directory
        let parentPID = getpid()
        let childDeadline = ProcessInfo.processInfo.systemUptime + 60
        try await #require(processExitsWith: .success, observing: [\.standardErrorContent], "Waiting for the fresh cache probe to exit") {
            [directory = directory as URL, parentPID = parentPID as pid_t, childDeadline = childDeadline as TimeInterval] in
            try await withExitDeadline("HtmlView", "probing the cache", by: childDeadline) {
                try await HtmlViewTests().probeCacheFromFreshProcess(directory, parentPID: parentPID)
            }
        }
        #expect(server.requests == Array(repeating: "/cache.svg", count: 4))
    }

    private func probeCacheFromFreshProcess(_ directory: URL, parentPID: pid_t) async throws {
        try #require(getpid() != parentPID)
        let server = try HtmlViewLoopback(directory: directory)
        let cache = cacheProbe(server, marker: "relaunched")
        let storage = storageProbe(write: false, marker: "relaunched found no storage")
        let mounted = try await CollectionWindow("""
        root = Col([HtmlView(\(try literal(cache)), 80), HtmlView(\(try literal(storage)), 80)], 0)
        """)
        defer { mounted.window.closeForTest() }
        try await waitFor("both relaunch probes mounted", in: mounted) { webViews(in: mounted.host).count == 2 }
        let views = webViews(in: mounted.host)
        let first = try #require(views.first)
        let second = try #require(views.last)
        try await loadCacheProbe(server, marker: "relaunched", in: first, mounted: mounted)
        try await requireText("relaunched found no storage", in: second)
    }

    private func cacheProbe(_ server: HtmlViewLoopback, marker: String) -> String {
        """
        <body><span id="status">ready \(marker)</span>
        <button style="position:absolute;top:30px;left:16px" onclick="loadImage()">Load image</button>
        <script>
        let loads = 0;
        function loadImage() {
            const image = new Image();
            image.onload = () => {
                document.querySelector('#status').textContent = 'loaded \(marker) ' + ++loads;
                document.body.append(image);
            };
            image.onerror = () => document.querySelector('#status').textContent = 'fixture image failed';
            image.src = '\(server.https.appending(path: "cache.svg").absoluteString)';
        }
        </script>
        """
    }

    private func loadCacheProbe(_ server: HtmlViewLoopback, marker: String, in web: WKWebView,
                                mounted: CollectionWindow, loads: Int = 1) async throws {
        try await requireText("ready \(marker)", in: web)
        let trust = try HtmlViewFixtureNavigation(webView: web, trust: (server.https, server.certificate))
        defer { trust.restore(in: web) }
        for load in 1...loads {
            try await clickPage(at: CGPoint(x: 49, y: 41), in: web, mounted: mounted)
            try await requireText("loaded \(marker) \(load)", in: web)
        }
    }

    @Test(.requiresDisplay, NativeTestWatchdog()) func navigationAndNewWindowsStayInTheAdmittedPage() async throws {
        let server = try await HtmlViewLoopback.start()
        defer { server.stop() }
        let base = server.http.absoluteString
        let mounted = try await mount("""
        <body><div id="status">admitted page</div>
        <a style="position:absolute;top:30px;left:16px" href="\(base)/clicked-top" target="_top"
            onclick="show('top link clicked')">Top link</a>
        <a style="position:absolute;top:55px;left:16px" href="\(base)/clicked-blank" target="_blank"
            onclick="show('blank link clicked')">Blank link</a>
        <a style="position:absolute;top:80px;left:16px" href="\(base)/clicked-self"
            onclick="show('self link clicked')">Self link</a>
        <form method="post" action="\(base)/form" style="position:absolute;top:105px;left:16px">
            <button onclick="show('form clicked')">Submit</button>
        </form>
        <script>
        function show(text) { document.querySelector('#status').textContent = text; }
        try { top.location = '\(base)/script-top'; } catch {}
        const popup = window.open('\(base)/script-popup');
        if (popup === null) show('script popup blocked');
        try { document.querySelector('form').submit(); } catch {}
        const nested = document.createElement('iframe');
        nested.style.display = 'none';
        nested.src = '\(base)/nested-frame';
        document.body.append(nested);
        setTimeout(() => { location.href = '\(base)/script-self'; }, 0);
        </script>
        """, height: 150)
        defer { mounted.window.closeForTest() }
        let web = try await webView(in: mounted)
        try await requireText("script popup blocked", in: web)
        let address = try #require(web.url)
        for (offset, marker) in [(40, "top link clicked"), (65, "blank link clicked"),
                                 (90, "self link clicked"), (116, "form clicked")] {
            try await clickPage(at: CGPoint(x: 41, y: offset - 1), in: web, mounted: mounted)
            try await requireText(marker, in: web)
        }
        // Attempt a top-level navigation even from the host's trusted WKWebView.
        let hostNavigation = server.http.appending(path: "host-navigation")
        var policy: WKNavigationActionPolicy?
        let observer = try HtmlViewFixtureNavigation(webView: web) { action, decision in
            if action.request.url == hostNavigation { policy = decision }
        }
        defer { observer.restore(in: web) }
        web.load(URLRequest(url: hostNavigation))
        try await waitFor("the host navigation policy decision", in: mounted) { policy != nil }
        try #require(policy == .cancel, "Host navigation policy: \(String(describing: policy)). \(diagnostics.description)")
        try await requireText("form clicked", in: web)
        #expect(web.url == address)
        #expect(webViews(in: mounted.host) == [web])
        #expect(server.requests.isEmpty)
    }

    @Test(.requiresDisplay, NativeTestWatchdog()) func onlyFiniteHeightFromThePageReachesSwift() async throws {
        let mounted = try await mount("""
        <body><div style="height:132px">checking page bridge</div>
        <button style="position:absolute;top:40px;left:16px" onclick="send(205)">Report height</button>
        <script>
        const handlers = window.webkit?.messageHandlers;
        let parentReadable = false;
        try { parentReadable = !!parent.webkit || !!parent.document; } catch {}
        if (!parentReadable && !handlers?.htmlViewHeight && (!handlers || !Reflect.ownKeys(handlers).length))
            document.querySelector('div').textContent = 'no callable page bridge';
        function send(height) {
            parent.postMessage({type: 'openui-intelligent-html-view-height', height}, '*');
        }
        addEventListener('load', () => {
            for (const height of [NaN, Infinity, -Infinity, "700", null]) send(height);
            parent.postMessage({type: 'something-else', height: 720}, '*');
        });
        </script>
        """)
        defer { mounted.window.closeForTest() }
        let web = try await webView(in: mounted)
        try await requireText("no callable page bridge", in: web)
        try await requireHeight(132, in: mounted)
        // The marker queues behind the forged message in the same page's
        // message queue. Wait for its reply before checking for a height relay.
        var forgeResult: Result<Any, any Error>?
        web.callAsyncJavaScript("""
        await new Promise(resolve => {
            addEventListener('message', function marker(event) {
                if (event.source !== window || event.data !== 'height test marker') return;
                removeEventListener('message', marker);
                resolve();
            });
            postMessage({type: 'openui-intelligent-html-view-height', height: 630}, '*');
            postMessage('height test marker', '*');
        });
        return true;
        """, arguments: [:], in: nil, in: .page) { forgeResult = $0 }
        try await waitFor("the wrapper to process the forged height message", in: mounted) { forgeResult != nil }
        let reply = try #require(forgeResult)
        try #require((try reply.get() as? Bool) == true)
        try await requireStable("rejecting the forged height (expected 132, saw \(mounted.frames["/children/0"]?.height.description ?? "missing"))",
                                in: mounted) {
            mounted.frames["/children/0"]?.height == 132 && diagnostics.track(web).lastHeight == 132
        }
        try await clickPage(at: CGPoint(x: 44, y: 49), in: web, mounted: mounted)
        try await requireHeight(205, in: mounted)
    }

    @Test(.requiresDisplay, NativeTestWatchdog()) func reportedHeightFollowsContentAndClampsBothEnds() async throws {
        let mounted = try await mount("""
        <body>
        <div id="panel" style="height:20px"></div>
        <button style="position:absolute;top:10px;left:10px;width:130px;height:32px"
            onclick="document.getElementById('panel').style.height = this.dataset.next;
                this.dataset.next = this.dataset.next === '180px' ? '2500px' : '20px'">Resize page</button>
        <script>document.querySelector('button').dataset.next = '180px'</script>
        """, height: 0)
        defer { mounted.window.closeForTest() }
        let web = try await webView(in: mounted)
        try await requireText("Resize page", in: web)
        try await requireHeight(80, in: mounted)
        for height in [CGFloat(180), 2400, 80] {
            try await clickPage(at: CGPoint(x: 64, y: 24), in: web, mounted: mounted)
            try await requireHeight(height, in: mounted)
        }
    }

    @Test(.requiresDisplay, NativeTestWatchdog()) func fixedHeightClampsAndIgnoresPageReports() async throws {
        let mounted = try await CollectionWindow("""
        $height = 1
        root = Col([HtmlView("<div style='height:120px'>short page</div>", $height), HtmlView("<div>second page</div>", 3000)], 0)
        """)
        defer { mounted.window.closeForTest() }
        try await requireHeight(80, in: mounted)
        try await requireHeight(2400, in: mounted, path: "/children/1")
        let views = webViews(in: mounted.host)
        let web = try await requireText("short page", in: try #require(views.first), try #require(views.last))
        try mounted.live.set("$height", to: .number(0))
        try await requireHeight(120, in: mounted)
        #expect(webViews(in: mounted.host).contains { $0 === web })
    }

    @Test(.requiresDisplay, NativeTestWatchdog()) func fixedPageThenFixedEmptyPageReturnsToInitialAutoHeight() async throws {
        let mounted = try await CollectionWindow("""
        $height = 150
        $html = "<div style='height:212px'>fixed page content</div>"
        root = Col([HtmlView($html, $height)], 0)
        """)
        defer { mounted.window.closeForTest() }
        let original = try await webView(in: mounted)
        try await requireText("fixed page content", in: original)
        try await requireHeight(150, in: mounted)
        try await waitFor("the original fixed page to finish loading", in: mounted) { !original.isLoading }

        try mounted.live.set("$html", to: .string(""))
        try await waitFor("empty page mounted", in: mounted) {
            guard let empty = webViews(in: mounted.host).first else { return false }
            return empty !== original && empty.url?.absoluteString == "about:blank" &&
                diagnostics.track(empty).frameAdmitted && !empty.isLoading
        }
        let empty = try await webView(in: mounted)
        try await requireHeight(150, in: mounted)

        try mounted.live.set("$height", to: .number(0))
        try await requireHeight(360, in: mounted)
        try await requireStable("empty page stays at auto height 360 (saw \(mounted.frames["/children/0"]?.height.description ?? "missing"))",
                                in: mounted) {
            mounted.frames["/children/0"]?.height == 360 && webViews(in: mounted.host) == [empty]
        }
    }

    @Test(.requiresDisplay, NativeTestWatchdog()) func streamingShowsPlaceholderAtInitialHeightUntilCompletion() async throws {
        let page = """
        root = Col([HtmlView("<body><script>document.body.append('completed interactive page')</script>")], 0)
        """
        let mounted = try await CollectionWindow(page, streaming: true)
        defer { mounted.window.closeForTest() }
        try await requireHeight(360, in: mounted)
        #expect(webViews(in: mounted.host).isEmpty)
        try await waitFor("native streaming placeholder is accessible", in: mounted) {
            accessibleStrings(in: mounted.host).contains("Preparing the interactive view…")
        }
        NativeTestWatchdog.waitingFor("Core to replace the HtmlView streaming placeholder")
        try await mounted.live.show(response: page, streaming: false)
        let web = try await webView(in: mounted)
        try await requireText("completed interactive page", in: web)
        try await requireHeight(80, in: mounted)
    }

    @Test(.requiresDisplay, NativeTestWatchdog()) func anOverflowingPageControlOnlyReceivesClicksInsideTheViewport() async throws {
        let mounted = try await mount("""
        <body style="overflow:hidden">
        <span id="result">waiting for click</span>
        <button style="position:absolute;top:42px;left:10px;width:120px;height:75px"
            onclick="document.getElementById('result').textContent = 'inside click received'">Tap me</button>
        """, height: 80)
        defer { mounted.window.closeForTest() }
        let web = try await webView(in: mounted)
        try await requireText("waiting for click", in: web)
        let frame = try mounted.rect("/children/0")
        #expect(frame.size == CGSize(width: 320, height: 80))
        try await clickPage(at: CGPoint(x: 49, y: 89), in: web, mounted: mounted, inside: false)
        try await requireText("waiting for click", in: web)
        let probe = diagnostics.track(web)
        try await requireStable("page text after the out-of-bounds click", in: mounted) {
            probe.readText(in: web)
            return probe.lastText?.contains("waiting for click") == true
        }
        try await clickPage(at: CGPoint(x: 49, y: 59), in: web, mounted: mounted)
        try await requireText("inside click received", in: web)
    }

    // On the display run, dismissal released the view while the looping process
    // survived the 120 s observation, using about one available core and 29 MiB.
    // WebContent exit was observed 0.111 s after the helper host exited. Allow
    // 10 s from the pre-exit marker for helper shutdown, scheduling and polling
    // on a loaded runner.
    @Test(.requiresDisplay, .timeLimit(.minutes(2)))
    func spinningPageKeepsHostResponsiveReleasesViewAndExitsWhenHostExits() async throws {
        try await verifyRunaway(Self.spinningScript, label: "spinning", requireExitOnDismissal: false,
                                progressing: Self.spinProgressed)
        try await requireSpinningProcessExitsWithHost()
    }

    private static let spinningScript = "while (true) {}"

    private static func spinProgressed(_ before: proc_taskinfo, _ after: proc_taskinfo) -> Bool {
        // pti_total_user counts Mach absolute time units (about 41.67 ns
        // each on Apple silicon), so convert before comparing with 50 ms.
        guard after.pti_total_user > before.pti_total_user else { return false }
        var timebase = mach_timebase_info_data_t()
        guard mach_timebase_info(&timebase) == KERN_SUCCESS, timebase.denom != 0 else { return false }
        let ticks = Double(after.pti_total_user - before.pti_total_user)
        return ticks * Double(timebase.numer) / Double(timebase.denom) > 50_000_000
    }

    private struct HostExitRecord: Codable {
        let host: pid_t
        let process: WebProcess
        let beforeExit: TimeInterval
    }

    @MainActor private final class WeakWebView {
        weak var value: WKWebView?
        init(_ value: WKWebView?) { self.value = value }
    }

    private func requireSpinningProcessExitsWithHost() async throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let report = directory.appending(path: "host-exit.json")
        let parent = getpid()
        let launchedAt = ProcessInfo.processInfo.systemUptime
        defer {
            if let data = try? Data(contentsOf: report),
               let record = try? JSONDecoder().decode(HostExitRecord.self, from: data) {
                record.process.terminate()
            }
        }

        try await #require(processExitsWith: .success, observing: [\.standardErrorContent],
                           "Waiting for the helper to start a spinning HtmlView and exit") {
            [report = report as URL, parent = parent as pid_t, childDeadline = (launchedAt + 60) as TimeInterval] in
            try await withExitDeadline("HtmlView", "starting a spinning page", by: childDeadline) {
                try await HtmlViewTests().exitHostWithSpinningPage(report: report, parent: parent)
            }
        }
        let observedHostExit = ProcessInfo.processInfo.systemUptime
        let record = try JSONDecoder().decode(HostExitRecord.self, from: Data(contentsOf: report))
        try #require(record.host != parent && record.beforeExit >= launchedAt && record.beforeExit <= observedHostExit,
                     "Expected a pre-exit timestamp from the helper host")
        try await waitForExit(of: record.process, by: record.beforeExit + 10,
                              waitingFor: "its exit within 10 s of the pre-exit marker from helper PID \(record.host)")
    }

    private func exitHostWithSpinningPage(report: URL, parent: pid_t) async throws {
        try #require(getpid() != parent, "Expected to run in the helper host")
        let mounted = try await mount(runawayPage(Self.spinningScript, label: "spinning"), height: 100)
        defer { mounted.window.closeForTest() }
        let web = try await webView(in: mounted)
        try await requireText("Start spinning", in: web)
        let pid = try processID(web)
        let process = try #require(WebProcess(pid))
        let watchdog = runawayWatchdog(process, until: ContinuousClock.now + .seconds(150))
        defer {
            watchdog.cancel()
            process.terminate()
        }
        try await clickPage(at: CGPoint(x: 54, y: 23), in: web, mounted: mounted)
        let before = try #require(processInfo(pid), "Expected CPU counters after starting the loop")
        try await requireProcessProgress("the helper's spinning page to consume CPU", pid: pid, before: before,
                                         in: mounted, progressing: Self.spinProgressed)
        try #require(process.isRunning && web.window === mounted.window, Comment(rawValue: diagnostics.description))
        let record = HostExitRecord(host: getpid(), process: process, beforeExit: ProcessInfo.processInfo.systemUptime)
        try JSONEncoder().encode(record).write(to: report, options: .atomic)
        // Keep the page attached so host exit triggers its process shutdown.
        withExtendedLifetime((mounted, web)) { Darwin.exit(EXIT_SUCCESS) }
    }

    @Test(.requiresDisplay) func allocatingPageLeavesHostResponsiveAndExitsWhenDismissed() async throws {
        try await verifyRunaway("""
        const retained = [];
        setInterval(() => {
            retained.push(new Uint8Array(2 * 1024 * 1024).fill(1));
        }, 50);
        """, label: "allocating", requireExitOnDismissal: true) { before, after in
            after.pti_resident_size > before.pti_resident_size + 12 * 1024 * 1024
        }
    }

    private func runawayPage(_ script: String, label: String) -> String {
        """
        <body><button style="position:absolute;top:12px;left:14px" onclick="run()">Start \(label)</button>
        <script>function run() { \(script) }</script>
        """
    }

    private func verifyRunaway(_ script: String, label: String, requireExitOnDismissal: Bool,
                               progressing: (proc_taskinfo, proc_taskinfo) -> Bool) async throws {
        let page = runawayPage(script, label: label)
        let program = """
        root = Col([Button("Host control"), HtmlView(\(try literal(page)), 80), HtmlView("<p>other page responsive</p>", 80)], 0)
        """
        var hostClicks = 0
        let mounted = try await CollectionWindow(program, component: { element, _ in
            guard element.typeName == "Button" else { return nil }
            return AnyView(Button("Host control") { hostClicks += 1 }.frame(width: 130, height: 30))
        })
        defer { mounted.window.closeForTest() }

        try await waitFor("both WebKit views mounted", in: mounted) { webViews(in: mounted.host).count == 2 }
        let (pid, other) = try await runawayProcesses(in: mounted, label: label)
        let before = try #require(processInfo(pid))
        let process = try #require(WebProcess(pid))
        let watchdogDeadline = ContinuousClock.now + .seconds(150)
        let watchdog = runawayWatchdog(process, until: watchdogDeadline)
        defer {
            watchdog.cancel()
            process.terminate()
        }
        do {
            let runaway = try #require(webViews(in: mounted.host).first { $0 !== other })
            try await clickPage(at: CGPoint(x: 54, y: 23), in: runaway, mounted: mounted)
        }
        try await requireProcessProgress("the \(label) script to consume resources in its WebContent process",
                                         pid: pid, before: before, in: mounted, progressing: progressing)
        let control = try mounted.rect("/children/0")
        try await mounted.click(CGPoint(x: control.midX, y: control.midY))
        try #require(hostClicks == 1)
        try await requireText("other page responsive", in: other)
        try #require(process.isRunning && webViews(in: mounted.host).count == 2)

        let dismissedView = WeakWebView(webViews(in: mounted.host).first { $0 !== other })
        try #require(dismissedView.value != nil)
        NativeTestWatchdog.waitingFor("Core to show the HtmlView dismissal program")
        try await mounted.live.show(response: """
        root = Col([Button("Host control"), Caption("closed"), HtmlView("<p>other page responsive</p>", 80)], 0)
        """, streaming: false)
        try await waitFor("dismissed WebKit view detached", in: mounted) { webViews(in: mounted.host) == [other] }
        if requireExitOnDismissal {
            try await waitForExit(of: process, by: ProcessInfo.processInfo.systemUptime + 10,
                                  waitingFor: "its exit within 10 s of dismissal")
            try #require(ContinuousClock.now < watchdogDeadline, "The process must exit before the test watchdog can terminate it")
        } else {
            try await waitFor("dismissed WebKit view to be released", in: mounted) { dismissedView.value == nil }
        }
        try await requireText("other page responsive", in: other)
    }

    private func requireProcessProgress(_ description: String, pid: pid_t, before: proc_taskinfo,
                                        in mounted: CollectionWindow,
                                        progressing: (proc_taskinfo, proc_taskinfo) -> Bool) async throws {
        var latest: proc_taskinfo?
        try await waitFor(description, in: mounted, timeout: .seconds(10), state: {
            let samples = latest.map { "CPU ticks \($0.pti_total_user), resident bytes \($0.pti_resident_size)" } ?? "unavailable"
            return "PID \(pid). Initial CPU ticks \(before.pti_total_user), initial resident bytes \(before.pti_resident_size). Last sample \(samples)."
        }) {
            latest = processInfo(pid)
            return latest.map { progressing(before, $0) } ?? false
        }
    }

    private func runawayWatchdog(_ process: WebProcess, until deadline: ContinuousClock.Instant) -> Task<Void, any Error> {
        // Run off the main actor in case AppKit input blocks. Only signal the
        // exact test process observed before the click.
        Task.detached {
            try await ContinuousClock().sleep(until: deadline)
            process.terminate()
        }
    }

    private func waitForExit(of process: WebProcess, by deadline: TimeInterval, waitingFor description: String) async throws {
        NativeTestWatchdog.beginWaiting(description)
        defer { NativeTestWatchdog.finishedWaiting() }
        while ProcessInfo.processInfo.systemUptime <= deadline {
            if process.hasExited && ProcessInfo.processInfo.systemUptime <= deadline { return }
            try await Task.sleep(for: .milliseconds(100))
        }
        throw CollectionInputFailure(description: "Timed out waiting for \(description) (WebContent PID \(process.pid), running: \(process.isRunning), exited: \(process.hasExited)). \(diagnostics.description)")
    }

    private func runawayProcesses(in mounted: CollectionWindow, label: String) async throws -> (pid_t, WKWebView) {
        let views = webViews(in: mounted.host)
        let first = try #require(views.first)
        let last = try #require(views.last)
        let runaway = try await requireText("Start \(label)", in: first, last)
        let other = try await requireText("other page responsive", in: first, last)
        let pid = try processID(runaway)
        let otherPID = try processID(other)
        try #require(pid != otherPID)
        return (pid, other)
    }

    @Test(.requiresDisplay, NativeTestWatchdog()) func terminatedPageStaysStoppedUntilExplicitReload() async throws {
        let mounted = try await mount("<p>original page content</p>", height: 130)
        defer { mounted.window.closeForTest() }
        let original = try await webView(in: mounted)
        try await requireText("original page content", in: original)
        let pid = try processID(original)
        let process = try #require(WebProcess(pid))
        try #require(process.terminate())
        try await waitFor("terminated page replaced with host controls", in: mounted) {
            webViews(in: mounted.host).isEmpty && accessibleStrings(in: mounted.host).contains("Interactive view stopped")
        }
        try await requireStable("the terminated page stays stopped and detached", in: mounted) {
            webViews(in: mounted.host).isEmpty && accessibleStrings(in: mounted.host).contains("Interactive view stopped")
        }
        try await requireHeight(130, in: mounted)
        let reload = try #require(mounted.accessibleButtons.first {
            ($0.accessibilityLabel?() ?? $0.accessibilityTitle?()) == "Reload"
        }, Comment(rawValue: diagnostics.description))
        try #require(reload.accessibilityPerformPress?() == true, Comment(rawValue: diagnostics.description))
        let replacement = try await webView(in: mounted)
        try await requireText("original page content", in: replacement)
        #expect(replacement !== original)
        #expect(try processID(replacement) != pid)
    }

    // WebKit has no public PID accessor. Use the test-only selector to
    // observe the one process owned by this fixture instead of matching any
    // system WebContent process by name.
    func processID(_ web: WKWebView) throws -> pid_t {
        try #require(observedWebProcessID(web), "Could not identify the test view's WebContent process")
    }

    func observedWebProcessID(_ web: WKWebView) -> pid_t? {
        let key = "_webProcessIdentifier"
        guard web.responds(to: NSSelectorFromString(key)),
              let number = web.value(forKey: key) as? NSNumber else { return nil }
        let pid = number.int32Value
        return pid > 1 && pid != getpid() ? pid : nil
    }

    struct WebProcess: Codable, Sendable {
        let pid: pid_t
        private let seconds: UInt64
        private let microseconds: UInt64

        init?(_ pid: pid_t) {
            guard let info = Self.info(pid) else { return nil }
            self.pid = pid
            seconds = info.pbi_start_tvsec
            microseconds = info.pbi_start_tvusec
        }

        var isRunning: Bool {
            guard let info = Self.info(pid) else { return false }
            return matchesRunningProcess(info)
        }

        var hasExited: Bool {
            if let info = Self.info(pid) {
                return !matchesRunningProcess(info)
            }
            return kill(pid, 0) == -1 && errno == ESRCH
        }

        private func matchesRunningProcess(_ info: proc_bsdinfo) -> Bool {
            info.pbi_start_tvsec == seconds && info.pbi_start_tvusec == microseconds && info.pbi_status != UInt32(SZOMB)
        }

        @discardableResult func terminate() -> Bool { isRunning && kill(pid, SIGKILL) == 0 }

        private static func info(_ pid: pid_t) -> proc_bsdinfo? {
            var info = proc_bsdinfo()
            let size = Int32(MemoryLayout<proc_bsdinfo>.size)
            return proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size ? info : nil
        }
    }

    nonisolated func processInfo(_ pid: pid_t) -> proc_taskinfo? {
        var info = proc_taskinfo()
        let size = Int32(MemoryLayout<proc_taskinfo>.size)
        return proc_pidinfo(pid, PROC_PIDTASKINFO, 0, &info, size) == size ? info : nil
    }

    @Test(.requiresDisplay, NativeTestWatchdog()) func largeDocumentPreparationKeepsHostControlResponsive() async throws {
        var hostClicks = 0
        let mounted = try await CollectionWindow("""
        $html = "<p>initial page</p>"
        root = Col([Button("Host control"), HtmlView($html, 100)], 0)
        """, component: { element, _ in
            guard element.typeName == "Button" else { return nil }
            return AnyView(Button("Host control") { hostClicks += 1 }.frame(width: 130, height: 30))
        })
        defer { mounted.window.closeForTest() }
        let original = try await webView(in: mounted)
        try await requireText("initial page", in: original)
        let large = "<!--" + String(repeating: "&", count: 900 * 1024) + "--><p>large page loaded</p>"
        NativeTestWatchdog.waitingFor("Core to show the large HtmlView document")
        try await mounted.live.show(response: """
        root = Col([Button("Host control"), HtmlView(\(try literal(large)), 100)], 0)
        """, streaming: false)
        try #require(mounted.live.errors.isEmpty)
        let control = try mounted.rect("/children/0")
        try await mounted.click(CGPoint(x: control.midX, y: control.midY))
        #expect(hostClicks == 1)
        try await waitFor("large document's browser mounted", in: mounted) {
            webViews(in: mounted.host).first.map { $0 !== original } ?? false
        }
        let replacement = try await webView(in: mounted)
        try await requireText("large page loaded", in: replacement)
    }

    private func waitFor(_ description: String, in mounted: CollectionWindow, timeout: Duration = .seconds(5),
                         state: () -> String = { "" }, _ condition: () -> Bool) async throws {
        let ready = try await mounted.waitForLayout(description, for: timeout) {
            _ = webViews(in: mounted.host)
            return condition()
        }
        guard ready else {
            throw CollectionInputFailure(description: "Timed out waiting for \(description). \(diagnostics.description) \(state())")
        }
    }

    // Check for a late, unwanted browser update after a positive observation.
    private func requireStable(_ description: @autoclosure () -> String, in mounted: CollectionWindow,
                               _ condition: () -> Bool) async throws {
        let changed = try await mounted.waitForLayout("late browser updates while checking \(description())", for: .milliseconds(300)) { !condition() }
        if changed {
            throw CollectionInputFailure(description: "State changed while checking \(description()). \(diagnostics.description)")
        }
    }

    private func requireHeight(_ height: CGFloat, in mounted: CollectionWindow, path: String = "/children/0") async throws {
        try await waitFor("height \(height) at \(path)", in: mounted, state: {
            "Actual height: \(mounted.frames[path]?.height.description ?? "missing")."
        }) {
            mounted.frames[path]?.height == height
        }
    }

    private func accessibleStrings(in host: NSView) -> [String] {
        var strings: [String] = []
        var pending = [host as AnyObject]
        while let item = pending.popLast() {
            if let node = item as? any NSAccessibilityProtocol, let value = node.accessibilityValue() as? String {
                strings.append(value)
            }
            if let label = item.accessibilityLabel?() { strings.append(label) }
            if let title = item.accessibilityTitle?() { strings.append(title) }
            pending.append(contentsOf: (item.accessibilityChildren?() ?? []).map { $0 as AnyObject })
        }
        return strings
    }

    func mount(_ html: String, height: Int? = nil) async throws -> CollectionWindow {
        try await CollectionWindow("root = Col([HtmlView(\(try literal(html)), \(height.map(String.init) ?? "null"))], 0)")
    }

    private func literal(_ string: String) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = .withoutEscapingSlashes
        return String(decoding: try encoder.encode(string), as: UTF8.self)
    }

    func webView(in mounted: CollectionWindow) async throws -> WKWebView {
        try await waitFor("the WebKit view to mount", in: mounted) { !webViews(in: mounted.host).isEmpty }
        return try #require(webViews(in: mounted.host).first, Comment(rawValue: diagnostics.description))
    }

    func webViews(in host: NSView) -> [WKWebView] {
        var views: [WKWebView] = []
        var pending = [host]
        while let view = pending.popLast() {
            if let web = view as? WKWebView {
                _ = diagnostics.track(web)
                views.append(web)
            }
            else { pending.append(contentsOf: view.subviews.reversed()) }
        }
        return views
    }

    @discardableResult
    func requireText(_ text: String, in views: WKWebView...) async throws -> WKWebView {
        NativeTestWatchdog.beginWaiting("WebKit to return text \(String(reflecting: text))")
        defer { NativeTestWatchdog.finishedWaiting() }
        let reads = views.map { web in
            let probe = diagnostics.track(web)
            return (web: web, probe: probe, revision: probe.textRevision)
        }
        let clock = ContinuousClock()
        let deadline = clock.now + .seconds(5)
        while true {
            try Task.checkCancellation()
            if let match = reads.first(where: { $0.probe.textRevision > $0.revision && $0.probe.lastText?.contains(text) == true }) {
                return match.web
            }
            let now = clock.now
            guard now < deadline else { break }
            for read in reads { read.probe.readText(in: read.web) }
            try await clock.sleep(until: min(deadline, now + .milliseconds(100)))
        }
        throw CollectionInputFailure(description: "Timed out waiting for web text \(String(reflecting: text)). \(diagnostics.description)")
    }

    // Resolve AppKit's actual target before delivering native input to WebKit.
    // The iframe fills the wrapper, so its CSS origin is the web view's top left.
    func clickPage(at point: CGPoint, in web: WKWebView, mounted: CollectionWindow,
                           inside: Bool = true) async throws {
        NativeTestWatchdog.waitingFor("AppKit to lay out HtmlView before clicking at \(point)")
        mounted.host.layoutSubtreeIfNeeded()
        NativeTestWatchdog.waitingFor("AppKit to dispatch the HtmlView click at \(point)")
        let bounds = web.bounds
        let local = CGPoint(x: bounds.minX + point.x,
                            y: web.isFlipped ? bounds.minY + point.y : bounds.maxY - point.y)
        let host = mounted.host
        let location = web.convert(local, to: nil)
        let target = host.hitTest(host.superview?.convert(location, from: nil) ?? location)
        let hitsWeb = target === web || target?.isDescendant(of: web) == true
        guard hitsWeb == inside else {
            throw CollectionInputFailure(description: "Page click at \(point) in \(bounds) hit \(String(describing: target)); expected inside=\(inside). \(diagnostics.description)")
        }
        if inside { mounted.window.makeFirstResponder(web) }
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = try #require(NSEvent.mouseEvent(with: type, location: location,
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: mounted.window.windowNumber, context: nil, eventNumber: 0,
                clickCount: 1, pressure: type == .leftMouseDown ? 1 : 0), Comment(rawValue: diagnostics.description))
            if !inside { mounted.window.sendEvent(event) }
            else if type == .leftMouseDown { web.mouseDown(with: event) }
            else { web.mouseUp(with: event) }
        }
    }

    @MainActor private final class BrowserDiagnostics {
        private var probes: [Probe] = []

        func track(_ web: WKWebView) -> Probe {
            if let probe = probes.first(where: { $0.web === web }) { return probe }
            let probe = Probe(web)
            probes.append(probe)
            return probe
        }

        var description: String {
            guard !probes.isEmpty else { return "No WebKit view observed." }
            return probes.enumerated().map { "browser \($0.offset + 1): \($0.element.description)" }.joined(separator: " | ")
        }

        @MainActor final class Probe {
            weak var web: WKWebView?
            private let coordinator: HtmlViewBrowser.Coordinator?
            private var reading = false
            private(set) var textRevision = 0
            private(set) var lastText: String?
            private var textFailure: NSError?
            var frameAdmitted: Bool { coordinator?.documentFrame != nil }
            var lastHeight: Double? { coordinator?.lastHeight }

            init(_ web: WKWebView) {
                self.web = web
                coordinator = web.navigationDelegate as? HtmlViewBrowser.Coordinator
            }

            func readText(in web: WKWebView) {
                guard !reading, let frame = coordinator?.documentFrame else { return }
                reading = true
                web.evaluateJavaScript("document.body?.innerText", in: frame, in: .defaultClient) { [weak self] result in
                    guard let self else { return }
                    reading = false
                    switch result {
                    case let .success(value as String):
                        lastText = value
                        textRevision += 1
                    case .success:
                        break
                    case let .failure(error):
                        textFailure = error as NSError
                    }
                }
            }

            var description: String {
                func error(_ value: NSError?) -> String {
                    value.map { "\($0.domain)(\($0.code))" } ?? "none"
                }
                let height = coordinator?.lastHeight.map { String(describing: $0) } ?? "none"
                let text = lastText.map { String(reflecting: String($0.prefix(300))) } ?? "none"
                return """
                    provisional failure \(error(coordinator?.lastProvisionalFailure)), navigation failure \(error(coordinator?.lastNavigationFailure)), \
                    height relay error \(error(coordinator?.lastRelayFailure)), WebContent terminated \(coordinator?.contentProcessTerminated ?? false), \
                    URL \(web?.url?.absoluteString ?? "none"), loading \(String(describing: web?.isLoading)), \
                    frame admitted \(frameAdmitted), last height \(height), last text \(text), text read error \(error(textFailure))
                    """
            }
        }
    }

}
}

@Suite(NativeTestWatchdog()) struct HtmlViewDocumentTests {
    @Test func upstreamPolicyAndBasePagePrecedeModelHTML() {
        let markup = #"<script>document.body.append("hello & goodbye")</script>"#
        let page = HtmlViewDocument(html: markup, dark: true).page
        let expectedPolicy = """
        default-src 'none'; script-src 'unsafe-inline' https://cdn.jsdelivr.net https://unpkg.com https://cdnjs.cloudflare.com https://esm.sh; style-src 'unsafe-inline' https://cdn.jsdelivr.net https://unpkg.com https://cdnjs.cloudflare.com https://esm.sh https://fonts.googleapis.com; font-src https://fonts.gstatic.com data:; img-src data: blob: https:; media-src data: blob:; connect-src https://cdn.jsdelivr.net https://unpkg.com https://cdnjs.cloudflare.com https://esm.sh; frame-src 'none'; form-action 'none'; base-uri 'none'
        """
        #expect(page.hasPrefix("""
        <!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="\(expectedPolicy)"><meta name="viewport" content="width=device-width,initial-scale=1"><style>:root{color-scheme:dark}html,body{margin:0}body{font:14px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif}</style>
        """))
        #expect(page.hasSuffix(markup))
        let lightPage = HtmlViewDocument(html: "", dark: false).page
        #expect(lightPage.contains(":root{color-scheme:light}"))
    }

    @Test @MainActor func fixedHeightAndFullWidthInAnAlignedColumn() async throws {
        let picture = try await LayoutPicture("""
        root = Col([HtmlView("<p>Short page</p>", 46)], 0, "start")
        """)
        #expect(try picture.rect("/children/0").size == CGSize(width: 240, height: 80))
    }

    @Test func modelMarkupStaysInsideTheSandboxedFrame() throws {
        let markup = #"</iframe><script>document.body.append("&quot;")</script>"#
        let wrapper = try HtmlViewDocument(html: markup, dark: false).wrapper()
        #expect(wrapper.contains(#"<iframe sandbox="allow-scripts" referrerpolicy="no-referrer""#))
        #expect(wrapper.contains(#"&lt;/iframe&gt;&lt;script&gt;document.body.append(&quot;&amp;quot;&quot;)&lt;/script&gt;"#))
        #expect(!wrapper.contains(markup))
    }
}
